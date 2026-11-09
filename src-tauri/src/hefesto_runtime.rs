//! Runtime nativo del dispositivo Hefesto: estado compartido, bucle de
//! latidos/sondeo y comandos IPC. Vive ligado a la app y muere con ella.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atlas_mac::{Clock, HefestoClient, OpaqueId, Phase, Roots, Runtime, Snapshot};
use tauri::{AppHandle, Emitter, Manager, State};

/// Cadencia del bucle: como mucho un viaje de red por vuelta.
const HEFESTO_TICK: Duration = Duration::from_millis(250);
/// Menor que la lease (15 s): un cuelgue puntual no la agota.
const HEFESTO_TIMEOUT: Duration = Duration::from_secs(5);
const HEFESTO_DEFAULT_URL: &str = "https://atlas-hub.local:8767";
/// Suspensión pendiente de reactivación manual: mientras exista, abrir la app
/// no vuelve a habilitar la ejecución por su cuenta.
const HEFESTO_SUSPEND_MARKER: &str = "hefesto-suspended";
const HEFESTO_CONFIG_DIR: &str = "hefesto";
const HEFESTO_CA_FILE: &str = "ca.pem";
const HEFESTO_CERT_FILE: &str = "device.pem";
const HEFESTO_KEY_FILE: &str = "device-key.pem";
const HEFESTO_DEVICE_ID_FILE: &str = "device-id";
const HEFESTO_URL_FILE: &str = "url";

#[derive(Clone, serde::Serialize)]
struct HefestoEvent {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(flatten)]
    snapshot: Snapshot,
}

enum HefestoCommand {
    Resume,
    Abort,
}

/// Estado compartido entre los comandos IPC y el hilo del dispositivo.
pub(crate) struct Hefesto {
    snapshot: Arc<Mutex<Snapshot>>,
    commands: Arc<Mutex<Option<mpsc::Sender<HefestoCommand>>>>,
}

impl Hefesto {
    pub(crate) fn new() -> Self {
        Self {
            snapshot: Arc::new(Mutex::new(Snapshot::unconfigured())),
            commands: Arc::new(Mutex::new(None)),
        }
    }

    fn current(&self) -> Snapshot {
        match self.snapshot.lock() {
            Ok(snapshot) => snapshot.clone(),
            Err(_) => Snapshot::failed("internal"),
        }
    }

    fn store(&self, snapshot: Snapshot) {
        if let Ok(mut slot) = self.snapshot.lock() {
            *slot = snapshot;
        }
    }

    fn send(&self, command: HefestoCommand) {
        if let Ok(slot) = self.commands.lock() {
            if let Some(sender) = slot.as_ref() {
                let _ = sender.send(command);
            }
        }
    }

    pub(crate) fn start(&self, app: AppHandle) {
        let snapshot = Arc::clone(&self.snapshot);
        let commands = Arc::clone(&self.commands);
        let spawned = thread::Builder::new()
            .name("hefesto".to_string())
            .spawn(move || {
                // Credenciales, TLS y resolución del nombre fuera del arranque
                // de la ventana: la consulta mDNS puede tardar y la app no
                // debe esperar a que termine.
                let (client, device_id) = match hefesto_transport(&app) {
                    Ok(transport) => transport,
                    Err(state) => {
                        if let Ok(mut slot) = snapshot.lock() {
                            *slot = state;
                        }
                        return;
                    }
                };
                if let Ok(mut slot) = snapshot.lock() {
                    *slot = Snapshot::connecting(&device_id);
                }
                // Si las raíces estándar no se pudieran resolver, el
                // dispositivo sigue vivo (eco y latido) pero toda lectura se
                // rechaza por falta de raíces.
                let roots = Roots::macos_default().unwrap_or_else(|_| Roots::new(Vec::new()));
                let marker = app
                    .path()
                    .app_data_dir()
                    .ok()
                    .map(|dir| dir.join(HEFESTO_SUSPEND_MARKER));
                let mut runtime = Runtime::new(client, device_id, roots);
                // Los puentes reales necesitan el `AppHandle` para reenviar
                // todo acceso a AppKit/CG al hilo principal. Solo en macOS.
                #[cfg(target_os = "macos")]
                {
                    runtime = runtime
                        .with_app_bridge(Arc::new(crate::hefesto_apps::WorkspaceApps::new(
                            app.clone(),
                        )))
                        .with_keyboard_bridge(Arc::new(crate::hefesto_keyboard::CgKeyboard::new(
                            app.clone(),
                        )))
                        .with_window_bridge(Arc::new(crate::hefesto_windows::AxWindows::new(
                            app.clone(),
                        )))
                        .with_system_bridge(Arc::new(crate::hefesto_system::MacSystem))
                        .with_terminal_bridge(Arc::new(crate::hefesto_terminal::MacTerminal::new()))
                        .with_capture_bridge(Arc::new(crate::hefesto_capture::MacCapture::new(
                            app.clone(),
                        )));
                }
                hefesto_loop(runtime, Instant::now(), snapshot, commands, app, marker)
            });
        if spawned.is_err() {
            self.store(Snapshot::failed("thread_failed"));
        }
    }
}

/// Ruta de entorno si no está vacía; si no, la ruta del fichero dentro de la
/// configuración persistente de la app (para CA, certificado y clave).
pub(crate) fn config_path(env: Option<String>, dir: Option<&Path>, file: &str) -> Option<String> {
    env.filter(|value| !value.trim().is_empty()).or_else(|| {
        let path = dir?.join(file);
        path.is_file().then(|| path.to_string_lossy().into_owned())
    })
}

/// Valor de entorno si no está vacío; si no, el contenido recortado del
/// fichero (para el identificador del dispositivo y la URL).
pub(crate) fn config_value(env: Option<String>, dir: Option<&Path>, file: &str) -> Option<String> {
    env.filter(|value| !value.trim().is_empty()).or_else(|| {
        let text = std::fs::read_to_string(dir?.join(file)).ok()?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_owned())
    })
}

/// La clave privada no debe ser legible por grupo ni por otros usuarios.
#[cfg(unix)]
pub(crate) fn private_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o077 == 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub(crate) fn private_file(_path: &Path) -> bool {
    true
}

/// Credenciales por entorno o, si faltan, desde
/// `~/Library/Application Support/<bundle>/hefesto/` (`ca.pem`, `device.pem`,
/// `device-key.pem`, `device-id` y, opcional, `url`). Los ficheros viven fuera
/// del repo; el entorno tiene prioridad para desarrollo y pruebas.
fn hefesto_transport(app: &AppHandle) -> std::result::Result<(HefestoClient, OpaqueId), Snapshot> {
    let dir = app
        .path()
        .app_data_dir()
        .ok()
        .map(|path| path.join(HEFESTO_CONFIG_DIR));
    let path = |env: &str, file: &str| config_path(std::env::var(env).ok(), dir.as_deref(), file);
    let value = |env: &str, file: &str| config_value(std::env::var(env).ok(), dir.as_deref(), file);
    let (Some(ca_path), Some(cert_path), Some(key_path), Some(device_id)) = (
        path("ATLAS_HEFESTO_CA", HEFESTO_CA_FILE),
        path("ATLAS_HEFESTO_CERT", HEFESTO_CERT_FILE),
        path("ATLAS_HEFESTO_KEY", HEFESTO_KEY_FILE),
        value("ATLAS_HEFESTO_DEVICE_ID", HEFESTO_DEVICE_ID_FILE),
    ) else {
        return Err(Snapshot::unconfigured());
    };
    if !private_file(Path::new(&key_path)) {
        return Err(Snapshot::failed("credentials_unreadable"));
    }
    let url = value("ATLAS_HEFESTO_URL", HEFESTO_URL_FILE)
        .unwrap_or_else(|| HEFESTO_DEFAULT_URL.to_string());
    let read =
        |path: &str| std::fs::read(path).map_err(|_| Snapshot::failed("credentials_unreadable"));
    let ca = read(&ca_path)?;
    let cert = read(&cert_path)?;
    let key = read(&key_path)?;
    // `Identity::from_pem` espera certificado y clave en el mismo PEM.
    let mut identity = cert;
    identity.push(b'\n');
    identity.extend_from_slice(&key);
    let device_id = OpaqueId::new(device_id).map_err(|error| Snapshot::failed(error.code()))?;
    let client = HefestoClient::new(&url, &ca, &identity, HEFESTO_TIMEOUT)
        .map_err(|error| Snapshot::failed(error.code()))?;
    Ok((client, device_id))
}

fn hefesto_clock(start: Instant) -> Option<Clock> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis();
    Clock::new(u64::try_from(millis).ok()?, start.elapsed()).ok()
}

fn hefesto_loop(
    mut runtime: Runtime<HefestoClient>,
    start: Instant,
    snapshot: Arc<Mutex<Snapshot>>,
    commands: Arc<Mutex<Option<mpsc::Sender<HefestoCommand>>>>,
    app: AppHandle,
    marker: Option<PathBuf>,
) {
    let (sender, receiver) = mpsc::channel();
    if let Ok(mut slot) = commands.lock() {
        *slot = Some(sender);
    }
    // La app arranca disponible salvo que la suspensión anterior siga
    // pendiente: entonces el marcador manda y hace falta activar a mano.
    let mut active = !hefesto_is_suspended(marker.as_deref());
    let mut last_phase = runtime.snapshot().phase;
    let mut published = Snapshot::unconfigured();
    loop {
        while let Ok(command) = receiver.try_recv() {
            match command {
                HefestoCommand::Resume => {
                    active = true;
                    hefesto_clear_suspended(marker.as_deref());
                    if let Some(now) = hefesto_clock(start) {
                        runtime.resume(now);
                    }
                }
                // El aborto ya es efectivo en local; el tick reintenta el aviso.
                HefestoCommand::Abort => {
                    active = false;
                    hefesto_write_suspended(marker.as_deref());
                    runtime.abort();
                }
            }
        }
        if let Some(now) = hefesto_clock(start) {
            runtime.tick(now);
        }
        match hefesto_gate(last_phase, runtime.snapshot().phase, active) {
            GateAction::Resume => {
                if let Some(now) = hefesto_clock(start) {
                    runtime.resume(now);
                }
            }
            // Suspensión remota (Hub Panel/Telegram): queda pendiente de ti.
            GateAction::Suspend => {
                active = false;
                hefesto_write_suspended(marker.as_deref());
            }
            GateAction::Activate => {
                active = true;
                hefesto_clear_suspended(marker.as_deref());
            }
            GateAction::None => {}
        }
        last_phase = runtime.snapshot().phase;
        hefesto_publish(&runtime, &snapshot, &app, &mut published);
        thread::sleep(HEFESTO_TICK);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GateAction {
    None,
    Resume,
    Suspend,
    Activate,
}

/// Traduce una transición de fase en lo que hay que hacer con la intención
/// persistida. Un registro nuevo reactiva solo si el usuario la dejó activa;
/// una suspensión desde `Ready` es remota y exige activación manual.
pub(crate) fn hefesto_gate(previous: Phase, current: Phase, active: bool) -> GateAction {
    match current {
        Phase::Ready if !active => GateAction::Activate,
        Phase::Suspended if previous == Phase::Ready && active => GateAction::Suspend,
        Phase::Suspended if previous != Phase::Suspended && active => GateAction::Resume,
        _ => GateAction::None,
    }
}

/// Lectura conservadora: si el marcador no se puede consultar, se asume
/// suspendido; solo la ausencia clara lo deja activo.
pub(crate) fn hefesto_is_suspended(path: Option<&Path>) -> bool {
    let Some(path) = path else {
        return false;
    };
    match std::fs::metadata(path) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

pub(crate) fn hefesto_write_suspended(path: Option<&Path>) {
    let Some(path) = path else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, b"suspended\n");
}

pub(crate) fn hefesto_clear_suspended(path: Option<&Path>) {
    let Some(path) = path else {
        return;
    };
    let _ = std::fs::remove_file(path);
}

fn hefesto_publish(
    runtime: &Runtime<HefestoClient>,
    shared: &Mutex<Snapshot>,
    app: &AppHandle,
    published: &mut Snapshot,
) {
    let current = runtime.snapshot();
    if &current == published {
        return;
    }
    *published = current.clone();
    if let Ok(mut slot) = shared.lock() {
        *slot = current.clone();
    }
    let _ = app.emit(
        crate::EVENT_HEFESTO,
        HefestoEvent {
            kind: "hefesto.state_changed",
            snapshot: current,
        },
    );
}

#[tauri::command]
pub(crate) fn hefesto_status(hefesto: State<'_, Hefesto>) -> Snapshot {
    hefesto.current()
}

#[tauri::command]
pub(crate) fn hefesto_resume(hefesto: State<'_, Hefesto>) {
    hefesto.send(HefestoCommand::Resume);
}

#[tauri::command]
pub(crate) fn hefesto_abort(hefesto: State<'_, Hefesto>) {
    hefesto.send(HefestoCommand::Abort);
}
