//! Puente real de consultas del sistema (puerta 6, corte 1). Solo se ejecutan
//! binarios fijos con argumentos fijos, sin shell, con el entorno limpio, un
//! timeout de 2 s, grupo de procesos propio y tope de 16 KiB de salida.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use atlas_mac::system::{SystemBridge, SystemFailure};
use atlas_mac::wire::SystemAction;

const TIMEOUT: Duration = Duration::from_secs(2);
const MAX_OUTPUT_BYTES: usize = 16 * 1024;
const TRUNCATED: &str = "\n… (recortado)";
const SIGKILL: i32 = 9;

// Justificación: `kill` es una función C pública de POSIX; se usa solo con el
// PID del hijo que acabamos de crear, que además es líder de su grupo.
unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
}

struct CommandSpec {
    program: &'static str,
    args: &'static [&'static str],
}

fn spec(action: SystemAction) -> CommandSpec {
    match action {
        SystemAction::Os => CommandSpec {
            program: "/usr/bin/sw_vers",
            args: &[],
        },
        SystemAction::Disk => CommandSpec {
            program: "/bin/df",
            args: &["-kH", "/"],
        },
        SystemAction::Uptime => CommandSpec {
            program: "/usr/bin/uptime",
            args: &[],
        },
        SystemAction::Memory => CommandSpec {
            program: "/usr/bin/vm_stat",
            args: &[],
        },
    }
}

pub struct MacSystem;

impl SystemBridge for MacSystem {
    fn run(&self, action: SystemAction) -> std::result::Result<String, SystemFailure> {
        let spec = spec(action);
        run_command(spec.program, spec.args, TIMEOUT)
    }
}

fn run_command(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> std::result::Result<String, SystemFailure> {
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .current_dir("/")
        .process_group(0)
        .spawn()
        .map_err(|_| SystemFailure::Detail("no se pudo iniciar la consulta".to_owned()))?;
    let id = child.id() as i32;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            let _ = stdout
                .take((MAX_OUTPUT_BYTES + 1) as u64)
                .read_to_end(&mut buffer);
        }
        let truncated = buffer.len() > MAX_OUTPUT_BYTES;
        if truncated {
            // Demasiada salida: se corta y se mata el grupo para que no se
            // quede bloqueado escribiendo en una tubería que ya no se lee.
            // Justificación: `kill` público de POSIX sobre el grupo del hijo.
            unsafe {
                kill(-id, SIGKILL);
            }
        }
        let status = child.wait();
        let _ = tx.send((buffer, status, truncated));
    });
    match rx.recv_timeout(timeout) {
        Ok((mut buffer, status, truncated)) => {
            if !truncated && !status.map(|value| value.success()).unwrap_or(false) {
                return Err(SystemFailure::Detail("la consulta falló".to_owned()));
            }
            if buffer.len() > MAX_OUTPUT_BYTES {
                buffer.truncate(MAX_OUTPUT_BYTES);
            }
            let mut text = String::from_utf8_lossy(&buffer).into_owned();
            if truncated {
                text.push_str(TRUNCATED);
            }
            Ok(text)
        }
        Err(_) => {
            // Justificación: `kill` público de POSIX sobre el grupo del hijo;
            // el lector terminará al cerrarse la tubería y hará `wait`.
            unsafe {
                kill(-id, SIGKILL);
            }
            Err(SystemFailure::Detail(
                "la consulta no respondió a tiempo".to_owned(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cada_accion_usa_un_binario_y_argumentos_fijos() {
        let os = spec(SystemAction::Os);
        assert_eq!(os.program, "/usr/bin/sw_vers");
        assert!(os.args.is_empty());
        let disk = spec(SystemAction::Disk);
        assert_eq!(disk.program, "/bin/df");
        assert_eq!(disk.args, ["-kH", "/"]);
        let uptime = spec(SystemAction::Uptime);
        assert_eq!(uptime.program, "/usr/bin/uptime");
        assert!(uptime.args.is_empty());
        let memory = spec(SystemAction::Memory);
        assert_eq!(memory.program, "/usr/bin/vm_stat");
        assert!(memory.args.is_empty());
    }

    #[test]
    fn ejecuta_un_binario_fijo_y_devuelve_su_salida() {
        let text = run_command("/bin/echo", &["hola"], TIMEOUT).expect("salida");
        assert_eq!(text.trim(), "hola");
    }

    #[test]
    fn una_consulta_lenta_se_corta_y_no_queda_viva() {
        let started = std::time::Instant::now();
        let failure = run_command("/bin/sleep", &["5"], Duration::from_millis(100))
            .expect_err("debe agotar el timeout");
        assert_eq!(
            failure,
            SystemFailure::Detail("la consulta no respondió a tiempo".to_owned())
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
