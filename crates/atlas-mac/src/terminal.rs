//! Terminal general de Hefesto: argv directo, sin shell, con límites y
//! tarjeta por invocación. La validación léxica vive aquí; el puente real
//! comprueba además que el binario exista y sea ejecutable.

use crate::wire::{OutputPayload, TerminalPayload};
use std::path::{Component, Path};

pub const MAX_PROGRAM_BYTES: usize = 1024;
pub const MAX_ARG_BYTES: usize = 1024;
pub const MAX_ARGS: usize = 32;
pub const MAX_COMMAND_BYTES: usize = 4096;

const ALLOWED_ROOTS: [&str; 6] = [
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
    "/usr/local/bin",
    "/opt/homebrew/bin",
];

const DENIED: [&str; 21] = [
    "sh",
    "bash",
    "zsh",
    "csh",
    "tcsh",
    "fish",
    "dash",
    "ksh",
    "sudo",
    "su",
    "doas",
    "launchctl",
    "systemsetup",
    "shutdown",
    "reboot",
    "halt",
    "kill",
    "killall",
    "pkill",
    "env",
    "xargs",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalFailure {
    Stable(String),
    Detail(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalOutcome {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub trait TerminalBridge: Send + Sync {
    fn run(
        &self,
        program: &str,
        args: &[String],
    ) -> std::result::Result<TerminalOutcome, TerminalFailure>;
}

pub fn denied_name(name: &str) -> bool {
    DENIED.contains(&name)
}

/// Ruta absoluta, sin `.`/`..`, dentro de una raíz permitida y con un nombre
/// que no sea un intérprete de shell ni una vía de elevación.
pub fn validate_program(program: &str) -> std::result::Result<(), &'static str> {
    if program.is_empty()
        || program.len() > MAX_PROGRAM_BYTES
        || program.contains('\0')
        || !Path::new(program).is_absolute()
    {
        return Err("programa no válido");
    }
    let path = Path::new(program);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err("programa no válido");
    }
    let allowed = ALLOWED_ROOTS.iter().any(|root| path.starts_with(root));
    if !allowed {
        return Err("programa no permitido");
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("programa no válido")?;
    if denied_name(name) {
        return Err("programa no permitido");
    }
    Ok(())
}

pub fn validate_args(args: &[String]) -> std::result::Result<(), &'static str> {
    if args.len() > MAX_ARGS {
        return Err("demasiados argumentos");
    }
    let mut total = 0usize;
    for arg in args {
        if arg.len() > MAX_ARG_BYTES || arg.contains('\0') {
            return Err("argumentos no válidos");
        }
        total = total.saturating_add(arg.len());
    }
    if total > MAX_COMMAND_BYTES {
        return Err("comando demasiado largo");
    }
    Ok(())
}

pub fn command_line(program: &str, args: &[String]) -> String {
    let mut line = quote(program);
    for arg in args {
        line.push(' ');
        line.push_str(&quote(arg));
    }
    line
}

fn quote(value: &str) -> String {
    if value.is_empty()
        || value
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '\'' | '"' | '\\' | '$' | '`'))
    {
        format!("«{value}»")
    } else {
        value.to_owned()
    }
}

pub fn handle_terminal(
    bridge: &dyn TerminalBridge,
    payload: &TerminalPayload,
) -> crate::Result<OutputPayload> {
    validate_program(&payload.program)
        .and_then(|()| validate_args(&payload.args))
        .map_err(str::to_owned)
        .map_or_else(OutputPayload::text, |()| {
            match bridge.run(&payload.program, &payload.args) {
                Ok(outcome) => {
                    let mut text = format!("Salida (código {}):", outcome.code);
                    if !outcome.stdout.trim().is_empty() {
                        text.push('\n');
                        text.push_str(outcome.stdout.trim_end());
                    }
                    if !outcome.stderr.trim().is_empty() {
                        text.push_str("\n[stderr]\n");
                        text.push_str(outcome.stderr.trim_end());
                    }
                    if outcome.stdout.trim().is_empty() && outcome.stderr.trim().is_empty() {
                        text.push_str("\nsin salida");
                    }
                    OutputPayload::text(text)
                }
                Err(TerminalFailure::Stable(text)) => OutputPayload::text(text),
                Err(TerminalFailure::Detail(detail)) => {
                    OutputPayload::text(format!("no se pudo ejecutar el comando: {detail}"))
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeBridge {
        result: Mutex<std::result::Result<TerminalOutcome, TerminalFailure>>,
        seen: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl FakeBridge {
        fn new(result: std::result::Result<TerminalOutcome, TerminalFailure>) -> Self {
            Self {
                result: Mutex::new(result),
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl TerminalBridge for FakeBridge {
        fn run(
            &self,
            program: &str,
            args: &[String],
        ) -> std::result::Result<TerminalOutcome, TerminalFailure> {
            self.seen
                .lock()
                .expect("visto")
                .push((program.to_owned(), args.to_vec()));
            self.result.lock().expect("resultado").clone()
        }
    }

    fn payload(program: &str, args: &[&str]) -> TerminalPayload {
        TerminalPayload {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }

    fn text(output: OutputPayload) -> String {
        output.text_value().expect("texto").to_owned()
    }

    #[test]
    fn un_comando_valido_se_ejecuta_y_formatea() {
        let bridge = FakeBridge::new(Ok(TerminalOutcome {
            code: 0,
            stdout: "hola\n".to_owned(),
            stderr: String::new(),
        }));
        let output =
            handle_terminal(&bridge, &payload("/bin/ls", &["-la", "/tmp"])).expect("salida");
        assert_eq!(text(output), "Salida (código 0):\nhola");
        assert_eq!(
            *bridge.seen.lock().expect("visto"),
            vec![(
                "/bin/ls".to_owned(),
                vec!["-la".to_owned(), "/tmp".to_owned()]
            )]
        );
    }

    #[test]
    fn stderr_y_codigo_no_cero_se_muestran() {
        let bridge = FakeBridge::new(Ok(TerminalOutcome {
            code: 2,
            stdout: String::new(),
            stderr: "fallo\n".to_owned(),
        }));
        let output =
            handle_terminal(&bridge, &payload("/usr/bin/which", &["nada"])).expect("salida");
        assert_eq!(text(output), "Salida (código 2):\n[stderr]\nfallo");
    }

    #[test]
    fn sin_salida_es_texto_estable() {
        let bridge = FakeBridge::new(Ok(TerminalOutcome {
            code: 0,
            stdout: String::new(),
            stderr: String::new(),
        }));
        let output = handle_terminal(&bridge, &payload("/bin/true", &[])).expect("salida");
        assert_eq!(text(output), "Salida (código 0):\nsin salida");
    }

    #[test]
    fn fuera_de_raices_o_con_shell_se_rechaza_sin_ejecutar() {
        let bridge = FakeBridge::new(Ok(TerminalOutcome {
            code: 0,
            stdout: "no".to_owned(),
            stderr: String::new(),
        }));
        for program in ["/etc/passwd", "/bin/sh", "/usr/bin/sudo", "/bin/../bin/rm"] {
            let output = handle_terminal(&bridge, &payload(program, &[])).expect("salida");
            let value = text(output);
            assert!(value.starts_with("programa"), "{program}: {value:?}");
        }
        assert!(bridge.seen.lock().expect("visto").is_empty());
    }

    #[test]
    fn los_topes_de_argumentos_se_respetan() {
        assert!(validate_args(&vec!["x".to_owned(); MAX_ARGS + 1]).is_err());
        assert!(validate_args(&["x".repeat(MAX_ARG_BYTES + 1)]).is_err());
        assert!(validate_args(&["x".repeat(MAX_COMMAND_BYTES + 1)]).is_err());
        assert!(validate_args(&["ok".to_owned()]).is_ok());
    }

    #[test]
    fn el_fallo_del_puente_se_traduce() {
        let bridge = FakeBridge::new(Err(TerminalFailure::Detail("timeout".to_owned())));
        let output = handle_terminal(&bridge, &payload("/bin/ls", &[])).expect("salida");
        assert_eq!(text(output), "no se pudo ejecutar el comando: timeout");
        let bridge = FakeBridge::new(Err(TerminalFailure::Stable(
            "programa no permitido".to_owned(),
        )));
        let output = handle_terminal(&bridge, &payload("/bin/ls", &[])).expect("salida");
        assert_eq!(text(output), "programa no permitido");
    }

    #[test]
    fn la_linea_muestra_el_comando_exacto() {
        assert_eq!(
            command_line("/bin/ls", &["-la".to_owned(), "mi archivo".to_owned()]),
            "/bin/ls -la «mi archivo»"
        );
    }
}
