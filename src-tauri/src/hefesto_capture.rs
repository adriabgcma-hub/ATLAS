//! Captura de pantalla con OCR local (puerta 7). Usa `screencapture` con
//! argv fijo y Apple Vision para reconocer texto. La imagen no sale del Mac.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use atlas_mac::capture::{CaptureBridge, CaptureFailure, CaptureOutcome};
use atlas_mac::wire::CaptureTarget;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_core_graphics::{CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};
use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};
use objc2_vision::{
    VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
    VNRequestTextRecognitionLevel,
};
use tauri::AppHandle;

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);
/// Vision tarda en calentar la primera vez (carga de modelos): 30 s evita que
/// la primera captura tras abrir la app falle con «no respondió a tiempo».
const OCR_TIMEOUT: Duration = Duration::from_secs(30);
const SIGKILL: i32 = 9;
const PRIVATE_DENIED: &str =
    "falta el permiso de Grabación de pantalla; actívalo en Ajustes del Sistema › Privacidad y seguridad › Grabación de pantalla y reinicia Atlas";

// Justificación: `kill` es una función C pública de POSIX; se usa solo con el
// PID del hijo que acabamos de crear, que además es líder de su grupo.
unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
}

pub struct MacCapture {
    app: AppHandle,
    requested: AtomicBool,
}

impl MacCapture {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            requested: AtomicBool::new(false),
        }
    }

    fn on_main<T, F>(&self, work: F) -> Option<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        self.app
            .clone()
            .run_on_main_thread(move || {
                let _ = tx.send(work());
            })
            .ok()?;
        rx.recv_timeout(Duration::from_secs(5)).ok()
    }

    fn permission(&self) -> bool {
        if self
            .on_main(|| CGPreflightScreenCaptureAccess())
            .unwrap_or(false)
        {
            return true;
        }
        // El diálogo del sistema se pide como mucho una vez por proceso; el
        // primer hilo que gana el `swap` lo muestra, los demás no insisten.
        if self.requested.swap(true, Ordering::SeqCst) {
            return false;
        }
        let _ = self.on_main(|| CGRequestScreenCaptureAccess());
        false
    }
}

impl CaptureBridge for MacCapture {
    fn capture(
        &self,
        _target: CaptureTarget,
    ) -> std::result::Result<CaptureOutcome, CaptureFailure> {
        if !self.permission() {
            return Err(CaptureFailure::Stable(PRIVATE_DENIED.to_owned()));
        }
        let path = capture_path()
            .ok_or_else(|| CaptureFailure::Detail("no hay carpeta de capturas".to_owned()))?;
        run_screencapture(&path)?;
        let text = ocr(&path)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("captura.png")
            .to_owned();
        Ok(CaptureOutcome { file_name, text })
    }
}

fn capture_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let dir = PathBuf::from(home).join("Documents/Atlas/capturas");
    std::fs::create_dir_all(&dir).ok()?;
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let (year, month, day, hour, minute, second) = utc_parts(seconds);
    Some(dir.join(format!(
        "captura-{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}.png"
    )))
}

/// Descomposición UTC sin dependencias: días desde 1970-01-01 → fecha civil.
fn utc_parts(seconds: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (seconds / 86_400) as i64;
    let secs = seconds % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (
        year,
        m as u32,
        d as u32,
        (secs / 3_600) as u32,
        ((secs % 3_600) / 60) as u32,
        (secs % 60) as u32,
    )
}

fn run_screencapture(path: &Path) -> std::result::Result<(), CaptureFailure> {
    let path_text = path
        .to_str()
        .ok_or_else(|| CaptureFailure::Detail("ruta de captura no válida".to_owned()))?;
    let mut child = Command::new("/usr/sbin/screencapture")
        .args(["-x", "-t", "png", path_text])
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .current_dir("/")
        .process_group(0)
        .spawn()
        .map_err(|_| CaptureFailure::Detail("no se pudo iniciar la captura".to_owned()))?;
    let id = child.id() as i32;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(child.wait());
    });
    match rx.recv_timeout(CAPTURE_TIMEOUT) {
        Ok(Ok(status)) if status.success() => {
            let ok = std::fs::metadata(path)
                .map(|meta| meta.is_file() && meta.len() > 0)
                .unwrap_or(false);
            if ok {
                Ok(())
            } else {
                Err(CaptureFailure::Detail("la captura quedó vacía".to_owned()))
            }
        }
        Ok(_) => Err(CaptureFailure::Detail("la captura falló".to_owned())),
        Err(_) => {
            // Justificación: `kill` público de POSIX sobre el grupo del hijo.
            unsafe {
                kill(-id, SIGKILL);
            }
            Err(CaptureFailure::Detail(
                "la captura no respondió a tiempo".to_owned(),
            ))
        }
    }
}

fn ocr(path: &Path) -> std::result::Result<String, CaptureFailure> {
    let path = path.to_owned();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(recognize(&path));
    });
    match rx.recv_timeout(OCR_TIMEOUT) {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(error)) => Err(CaptureFailure::Detail(error)),
        Err(_) => Err(CaptureFailure::Detail(
            "el reconocimiento de texto no respondió a tiempo".to_owned(),
        )),
    }
}

fn recognize(path: &Path) -> std::result::Result<String, String> {
    let path = path
        .to_str()
        .ok_or_else(|| "ruta de captura no válida".to_owned())?;
    let path = NSString::from_str(path);
    let url = NSURL::fileURLWithPath(&path);
    let options: Retained<NSDictionary<VNImageOption, AnyObject>> = NSDictionary::new();
    let handler = unsafe {
        VNImageRequestHandler::initWithURL_options(VNImageRequestHandler::alloc(), &url, &options)
    };
    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    let languages =
        NSArray::from_retained_slice(&[NSString::from_str("es-ES"), NSString::from_str("en-US")]);
    request.setRecognitionLanguages(&languages);
    let results_request = request.clone();
    let request: Retained<VNRequest> = unsafe { Retained::cast_unchecked(request) };
    let requests = NSArray::from_retained_slice(&[request]);
    handler
        .performRequests_error(&requests)
        .map_err(|_| "no se pudo reconocer el texto".to_owned())?;
    let Some(results) = results_request.results() else {
        return Ok(String::new());
    };
    let mut lines = Vec::new();
    for observation in results.iter() {
        if let Some(text) = observation.topCandidates(1).iter().next() {
            lines.push(text.string().to_string());
        }
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::utc_parts;

    #[test]
    fn la_fecha_utc_se_descompone_sin_dependencias() {
        assert_eq!(utc_parts(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(utc_parts(1_700_000_000), (2023, 11, 14, 22, 13, 20));
    }
}
