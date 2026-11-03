//! Terminal general real (puerta 6): argv directo, sin shell, grupo de
//! procesos propio, timeout y salida acotada. La tarjeta del relay ya mostró
//! el comando exacto; aquí solo se ejecuta lo validado.

use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use atlas_mac::terminal::{validate_program, TerminalBridge, TerminalFailure, TerminalOutcome};

const TIMEOUT: Duration = Duration::from_secs(10);
const CAP_PER_PIPE: usize = 24 * 1024;
const TRUNCATED: &str = "\n… (recortado)";
const SIGKILL: i32 = 9;

// Justificación: `kill` es una función C pública de POSIX; se usa solo con el
// PID del hijo que acabamos de crear, que además es líder de su grupo.
unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
}

pub struct MacTerminal {
    timeout: Duration,
}

impl MacTerminal {
    pub fn new() -> Self {
        Self { timeout: TIMEOUT }
    }

    #[cfg(test)]
    fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl Default for MacTerminal {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalBridge for MacTerminal {
    fn run(
        &self,
        program: &str,
        args: &[String],
    ) -> std::result::Result<TerminalOutcome, TerminalFailure> {
        let canonical = std::fs::canonicalize(program)
            .map_err(|_| TerminalFailure::Stable("programa no encontrado".to_owned()))?;
        let canonical_text = canonical
            .to_str()
            .ok_or_else(|| TerminalFailure::Stable("programa no válido".to_owned()))?;
        validate_program(canonical_text)
            .map_err(|error| TerminalFailure::Stable(error.to_owned()))?;
        let metadata = std::fs::metadata(&canonical)
            .map_err(|_| TerminalFailure::Stable("programa no encontrado".to_owned()))?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err(TerminalFailure::Stable("programa no ejecutable".to_owned()));
        }
        run_command(canonical_text, args, self.timeout)
    }
}

fn read_capped<R: Read>(mut reader: R, id: i32) -> (Vec<u8>, bool) {
    let mut buffer = Vec::new();
    let mut overflow = Vec::new();
    let _ = reader
        .by_ref()
        .take(CAP_PER_PIPE as u64 + 1)
        .read_to_end(&mut buffer);
    let truncated = buffer.len() > CAP_PER_PIPE;
    if truncated {
        buffer.truncate(CAP_PER_PIPE);
        // Si el hijo sigue escribiendo, la tubería se llenaría y quedaría
        // bloqueado: se mata el grupo para cerrar el flujo.
        // Justificación: `kill` público de POSIX sobre el grupo del hijo.
        unsafe {
            kill(-id, SIGKILL);
        }
        // Drenar lo que quede para que el hijo pueda terminar.
        let _ = reader.read_to_end(&mut overflow);
    }
    (buffer, truncated)
}

fn text(bytes: Vec<u8>, truncated: bool) -> String {
    let mut value = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        value.push_str(TRUNCATED);
    }
    value
}

fn run_command(
    program: &str,
    args: &[String],
    timeout: Duration,
) -> std::result::Result<TerminalOutcome, TerminalFailure> {
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir("/")
        .process_group(0)
        .spawn()
        .map_err(|_| TerminalFailure::Detail("no se pudo iniciar el comando".to_owned()))?;
    let id = child.id() as i32;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| TerminalFailure::Detail("sin salida del comando".to_owned()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| TerminalFailure::Detail("sin errores del comando".to_owned()))?;
    let stdout_reader = thread::spawn(move || read_capped(stdout, id));
    let stderr_reader = thread::spawn(move || read_capped(stderr, id));
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status);
    });
    let status = match rx.recv_timeout(timeout) {
        Ok(status) => status,
        Err(_) => {
            // Justificación: `kill` público de POSIX sobre el grupo del hijo.
            unsafe {
                kill(-id, SIGKILL);
            }
            let _ = rx.recv_timeout(Duration::from_secs(2));
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(TerminalFailure::Detail(
                "el comando no respondió a tiempo".to_owned(),
            ));
        }
    };
    let (out, out_truncated) = stdout_reader
        .join()
        .map_err(|_| TerminalFailure::Detail("no se pudo leer la salida".to_owned()))?;
    let (err, err_truncated) = stderr_reader
        .join()
        .map_err(|_| TerminalFailure::Detail("no se pudo leer el error".to_owned()))?;
    let code = status.map(|value| value.code().unwrap_or(-1)).unwrap_or(-1);
    Ok(TerminalOutcome {
        code,
        stdout: text(out, out_truncated),
        stderr: text(err, err_truncated),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ejecuta_un_binario_fijo_y_devuelve_su_salida() {
        let bridge = MacTerminal::new();
        let outcome = bridge
            .run("/bin/echo", &["hola".to_owned()])
            .expect("salida");
        assert_eq!(outcome.code, 0);
        assert_eq!(outcome.stdout.trim(), "hola");
        assert!(outcome.stderr.is_empty());
    }

    #[test]
    fn un_binario_fuera_de_las_raices_se_rechaza() {
        let bridge = MacTerminal::new();
        let error = bridge.run("/etc/passwd", &[]).expect_err("rechazado");
        assert_eq!(
            error,
            TerminalFailure::Stable("programa no permitido".to_owned())
        );
    }

    #[test]
    fn una_shell_se_rechaza_aunque_este_en_una_raiz() {
        let bridge = MacTerminal::new();
        let error = bridge
            .run("/bin/sh", &["-c".to_owned(), "echo hola".to_owned()])
            .expect_err("rechazado");
        assert_eq!(
            error,
            TerminalFailure::Stable("programa no permitido".to_owned())
        );
    }

    #[test]
    fn un_timeout_mata_el_grupo_y_lo_dice() {
        let bridge = MacTerminal::with_timeout(Duration::from_millis(100));
        let error = bridge
            .run("/bin/sleep", &["5".to_owned()])
            .expect_err("timeout");
        assert_eq!(
            error,
            TerminalFailure::Detail("el comando no respondió a tiempo".to_owned())
        );
    }

    #[test]
    fn la_salida_se_corta_y_se_mata_el_grupo() {
        let bridge = MacTerminal::with_timeout(Duration::from_secs(5));
        let outcome = bridge.run("/usr/bin/yes", &[]).expect("salida");
        assert!(outcome.stdout.ends_with(TRUNCATED));
        assert!(outcome.stdout.len() <= CAP_PER_PIPE + TRUNCATED.len());
    }
}
