//! Puente real de control de apps de Hefesto (Task 4).
//!
//! Implementa [`AppBridge`] sobre `NSWorkspace`. AppKit exige que cierto
//! estado (`NSWorkspace`, `NSRunningApplication`) se consulte desde el hilo
//! principal: todo acceso se reenvía con `run_on_main_thread`. El hilo que
//! llama al puente (el poller de Hefesto) nunca toca AppKit directamente;
//! hace viajes cortos y acotados al hilo principal y espera fuera de él.
//!
//! Este módulo solo existe en macOS: el resto de plataformas no tienen
//! `NSWorkspace`.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use atlas_mac::{AppBridge, AppRef, CloseOutcome};
use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};
use objc2_foundation::{NSString, NSURL};
use tauri::AppHandle;

/// Viaje máximo al hilo principal para una lectura simple (`running`, `open`).
const MAIN_ROUNDTRIP: Duration = Duration::from_secs(5);
/// Viaje corto de la sonda de presencia durante el cierre acotado.
const PRESENCE_PROBE: Duration = Duration::from_millis(1_500);
/// `open` puede implicar arranque en frío: se le da más margen.
const OPEN_ROUNDTRIP: Duration = Duration::from_secs(10);
/// Ventana total de espera tras pedir el cierre (en el hilo llamante).
const QUIT_WINDOW: Duration = Duration::from_secs(5);
/// Paso entre sondas de presencia.
const QUIT_STEP: Duration = Duration::from_millis(250);

fn app_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from(home).join("Applications"),
    ]
}

/// Idiomas preferidos del usuario, leídos de sus preferencias globales sin
/// invocar `defaults` ni depender de AppKit.
fn preferred_languages() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else {
        return vec!["en".to_owned()];
    };
    let path = PathBuf::from(home).join("Library/Preferences/.GlobalPreferences.plist");
    let languages = plist::Value::from_file(path)
        .ok()
        .and_then(|value| {
            value
                .as_dictionary()?
                .get("AppleLanguages")?
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_string().map(str::to_owned))
                        .take(3)
                        .collect::<Vec<_>>()
                })
        })
        .unwrap_or_default();
    if languages.is_empty() {
        vec!["en".to_owned()]
    } else {
        languages
    }
}

/// Variantes de cada idioma: `es-AD` → `es-ad` y `es`; siempre añade `en`
/// como alias para que «Find My» funcione aunque la interfaz esté en español.
fn language_variants(languages: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for language in languages {
        let lower = language.to_lowercase().replace('_', "-");
        let base = lower.split('-').next().unwrap_or_default().to_owned();
        for variant in [lower, base] {
            if !variant.is_empty() && !out.contains(&variant) {
                out.push(variant);
            }
        }
    }
    if !out.iter().any(|variant| variant == "en") {
        out.push("en".to_owned());
    }
    out
}

fn localized_from_loctable(resources: &Path, variants: &[String]) -> Vec<String> {
    let Ok(value) = plist::Value::from_file(resources.join("InfoPlist.loctable")) else {
        return Vec::new();
    };
    let Some(table) = value.as_dictionary() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for variant in variants {
        let Some(entry) = table
            .iter()
            .find(|(key, _)| key.to_lowercase() == *variant)
            .map(|(_, value)| value)
            .and_then(plist::Value::as_dictionary)
        else {
            continue;
        };
        for key in ["CFBundleDisplayName", "CFBundleName"] {
            if let Some(name) = entry.get(key).and_then(plist::Value::as_string) {
                let name = name.trim();
                if !name.is_empty() && !out.iter().any(|seen| seen == name) {
                    out.push(name.to_owned());
                }
            }
        }
    }
    out
}

fn localized_from_lproj(resources: &Path, variants: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for variant in variants {
        let path = resources.join(format!("{variant}.lproj/InfoPlist.strings"));
        let Ok(value) = plist::Value::from_file(path) else {
            continue;
        };
        let Some(table) = value.as_dictionary() else {
            continue;
        };
        for key in ["CFBundleDisplayName", "CFBundleName"] {
            if let Some(name) = table.get(key).and_then(plist::Value::as_string) {
                let name = name.trim();
                if !name.is_empty() && !out.iter().any(|seen| seen == name) {
                    out.push(name.to_owned());
                }
            }
        }
    }
    out
}

/// Nombres localizados del paquete para los idiomas del usuario y el inglés.
fn localized_names(path: &Path) -> Vec<String> {
    let resources = path.join("Contents").join("Resources");
    let variants = language_variants(&preferred_languages());
    let mut names = localized_from_loctable(&resources, &variants);
    for name in localized_from_lproj(&resources, &variants) {
        if !names.iter().any(|seen| seen == &name) {
            names.push(name);
        }
    }
    names
}

/// Lee un paquete `.app` y extrae identificador y nombre del `Info.plist`.
/// Necesita `CFBundleIdentifier`; el nombre prioriza el localizado para el
/// idioma del usuario y, si no hay, `CFBundleDisplayName`, `CFBundleName` o el
/// tallo del fichero. El resto de nombres quedan como alias de búsqueda.
fn read_app(path: &Path) -> Option<AppRef> {
    let plist = path.join("Contents").join("Info.plist");
    let value = plist::Value::from_file(&plist).ok()?;
    let map = value.as_dictionary()?;
    let bundle_id = map.get("CFBundleIdentifier")?.as_string()?.to_string();
    let base = map
        .get("CFBundleDisplayName")
        .and_then(|v| v.as_string())
        .or_else(|| map.get("CFBundleName").and_then(|v| v.as_string()))
        .map(str::to_string)
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    let mut names = localized_names(path);
    let name = if let Some(first) = names.first() {
        first.clone()
    } else {
        base.clone()
    };
    let mut aliases = names.drain(..).skip(1).collect::<Vec<_>>();
    aliases.push(base);
    aliases.retain(|alias| !alias.eq_ignore_ascii_case(&name));
    Some(AppRef {
        name,
        bundle_id,
        url: path.to_string_lossy().into_owned(),
        aliases,
    })
}

/// Escaneo único de las carpetas de aplicaciones más las de usuario. Se apoya
/// solo en `Info.plist`, sin tocar APIs privadas de LaunchServices.
fn scan_installed() -> Vec<AppRef> {
    let mut out = Vec::new();
    for dir in app_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("app") {
                if let Some(app) = read_app(&path) {
                    out.push(app);
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Lista de apps corrientes visibles (política `Regular`). Solo desde el hilo
/// principal: `NSWorkspace` no garantiza consistencia en otros hilos.
fn collect_running() -> Vec<AppRef> {
    let workspace = NSWorkspace::sharedWorkspace();
    workspace
        .runningApplications()
        .to_vec()
        .into_iter()
        .filter(|app| app.activationPolicy() == NSApplicationActivationPolicy::Regular)
        .filter_map(|app| {
            let bundle_id = app.bundleIdentifier()?.to_string();
            let name = app
                .localizedName()
                .map(|value| value.to_string())
                .unwrap_or_default();
            let url = app
                .bundleURL()
                .and_then(|bundle| bundle.path())
                .map(|path| path.to_string())
                .unwrap_or_default();
            Some(AppRef {
                name,
                bundle_id,
                url,
                aliases: Vec::new(),
            })
        })
        .collect()
}

/// Comprueba si un identificador de paquete sigue entre las apps corrientes.
/// Solo desde el hilo principal y sin dormir: es la sonda del cierre acotado.
fn bundle_is_running(bundle_id: &str) -> bool {
    let workspace = NSWorkspace::sharedWorkspace();
    workspace.runningApplications().to_vec().iter().any(|app| {
        app.bundleIdentifier()
            .map(|value| value.to_string())
            .as_deref()
            == Some(bundle_id)
    })
}

/// Resultado del primer viaje al hilo principal en `terminate`.
enum TerminateKick {
    /// La app no estaba corriendo: no hay nada que cerrar ni que esperar.
    AlreadyGone,
    /// Se envió `terminate()` una vez; queda por sondear la salida.
    Signalled,
}

pub struct WorkspaceApps {
    handle: AppHandle,
    catalog: Mutex<Option<Vec<AppRef>>>,
}

impl WorkspaceApps {
    pub fn new(handle: AppHandle) -> Self {
        Self {
            handle,
            catalog: Mutex::new(None),
        }
    }

    /// Catálogo cacheado: se escanea una sola vez y se sirve desde memoria.
    fn catalog(&self) -> Vec<AppRef> {
        let mut guard = self
            .catalog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_none() {
            *guard = Some(scan_installed());
        }
        guard.clone().unwrap_or_default()
    }

    /// Sonda de presencia en el hilo principal (viaje corto con timeout
    /// propio). Un `true` conservador si el hilo principal no responde, para
    /// no declarar cerrado algo que puede seguir vivo.
    fn probe_running(&self, bundle_id: &str) -> bool {
        let (tx, rx) = mpsc::channel();
        let bundle_id = bundle_id.to_string();
        let handle = self.handle.clone();
        let forwarded = handle.run_on_main_thread(move || {
            let _ = tx.send(bundle_is_running(&bundle_id));
        });
        if forwarded.is_err() {
            return true;
        }
        rx.recv_timeout(PRESENCE_PROBE).unwrap_or(true)
    }
}

impl AppBridge for WorkspaceApps {
    fn installed(&self) -> Vec<AppRef> {
        self.catalog()
    }

    fn running(&self) -> Vec<AppRef> {
        let (tx, rx) = mpsc::channel();
        let handle = self.handle.clone();
        let forwarded = handle.run_on_main_thread(move || {
            let _ = tx.send(collect_running());
        });
        if forwarded.is_err() {
            return Vec::new();
        }
        rx.recv_timeout(MAIN_ROUNDTRIP).unwrap_or_default()
    }

    fn open(&self, target: &AppRef) -> std::result::Result<(), String> {
        let (tx, rx) = mpsc::channel();
        let handle = self.handle.clone();
        let url = target.url.clone();
        handle
            .run_on_main_thread(move || {
                // Todo el trabajo AppKit va en el hilo principal. `NSString`
                // e `NSURL` se construyen y consumen aquí; nunca cruzan el
                // canal (solo viaja el `Result` plano). Las funciones del
                // binding `objc2` usadas aquí (`from_str`, `fileURLWithPath`,
                // `openURL`) son `pub fn` seguras en la 0.3 fijada: no hay
                // bloque `unsafe` que justificar.
                let path = NSString::from_str(&url);
                let file_url = NSURL::fileURLWithPath(&path);
                let opened = NSWorkspace::sharedWorkspace().openURL(&file_url);
                let _ = tx.send(if opened {
                    Ok(())
                } else {
                    Err("NSWorkspace rechazó la apertura".to_string())
                });
            })
            .map_err(|_| "no se pudo reenviar al hilo principal".to_string())?;
        rx.recv_timeout(OPEN_ROUNDTRIP)
            .map_err(|_| "timeout del hilo principal".to_string())?
    }

    fn terminate(&self, target: &AppRef) -> std::result::Result<CloseOutcome, String> {
        // 1) Pedir el cierre UNA sola vez en el hilo principal y volver de
        //    inmediato: si durmiéramos aquí, el runloop no procesaría las
        //    notificaciones distribuidas de `NSWorkspace` y la lista de apps
        //    no se refrescaría (falso `StillPresent`).
        let (tx, rx) = mpsc::channel();
        let handle = self.handle.clone();
        let bundle_id = target.bundle_id.clone();
        handle
            .run_on_main_thread(move || {
                let workspace = NSWorkspace::sharedWorkspace();
                let found = workspace
                    .runningApplications()
                    .to_vec()
                    .into_iter()
                    .find(|app| {
                        app.bundleIdentifier()
                            .map(|value| value.to_string())
                            .as_deref()
                            == Some(bundle_id.as_str())
                    });
                let Some(app) = found else {
                    // No estaba corriente: se informa sin afirmar que se cerró.
                    let _ = tx.send(TerminateKick::AlreadyGone);
                    return;
                };
                // Dispara el cierre suave y regresa; la confirmación de salida
                // la hace el hilo llamante con sondas cortas.
                let _ = app.terminate();
                let _ = tx.send(TerminateKick::Signalled);
            })
            .map_err(|_| "no se pudo reenviar al hilo principal".to_string())?;

        match rx.recv_timeout(MAIN_ROUNDTRIP) {
            Ok(TerminateKick::AlreadyGone) => return Ok(CloseOutcome::NotRunning),
            Ok(TerminateKick::Signalled) => {}
            Err(_) => return Err("timeout del hilo principal".to_string()),
        }

        // 2) Espera acotada EN EL HILO LLAMANTE (no en el principal): cada
        //    sonda es un viaje corto al hilo principal con su propio timeout,
        //    así el runloop sigue girando y `runningApplications` se refresca.
        let deadline = Instant::now() + QUIT_WINDOW;
        loop {
            std::thread::sleep(QUIT_STEP);
            if !self.probe_running(&target.bundle_id) {
                return Ok(CloseOutcome::Gone);
            }
            if Instant::now() >= deadline {
                return Ok(CloseOutcome::StillPresent);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_idiomas_incluyen_la_variante_base_y_el_ingles() {
        let variants = language_variants(&["es-AD".to_owned(), "en-US".to_owned()]);
        assert_eq!(variants, vec!["es-ad", "es", "en-us", "en"]);
    }

    #[test]
    fn findmy_expone_su_nombre_localizado() {
        let Some(app) = read_app(Path::new("/System/Applications/FindMy.app")) else {
            return;
        };
        if preferred_languages()
            .iter()
            .any(|language| language.to_lowercase().starts_with("es"))
        {
            assert!(
                app.name == "Buscar" || app.aliases.iter().any(|alias| alias == "Buscar"),
                "nombre={} alias={:?}",
                app.name,
                app.aliases
            );
        }
    }
}
