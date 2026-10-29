//! Capa Tauri de Atlas (Fase 3): ventana, atajo global y comandos IPC.
//!
//! Fina por diseño: el cerebro (sesiones, Groq, Telegram) vive en el relay
//! Python. Esta capa transporta peticiones y eventos, sin estado propio.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atlas_core::{AtlasError, Source, codes, get_events, get_session, post_cancel, post_send};
use atlas_mac::{Clock, HefestoClient, OpaqueId, Phase, Runtime, Snapshot};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

/// URL del cerebro en la red local. No es un secreto. Por nombre (.local):
/// la IP del servidor puede cambiar, el nombre no.
fn default_relay_url() -> String {
    std::env::var("ATLAS_RELAY_URL").unwrap_or_else(|_| "http://atlas-hub.local:8765".to_string())
}

struct CoreState {
    relay_url: String,
    relay_cursor: Arc<Mutex<u64>>,
}

fn parse_source(raw: Option<String>) -> Result<&'static str, AtlasError> {
    match raw.as_deref() {
        None | Some("local") => Ok("local"),
        Some("telegram") => Ok("telegram"),
        Some(_) => Err(AtlasError::new(
            codes::INVALID_SOURCE,
            "Origen desconocido.",
            false,
        )),
    }
}

/// Canales IPC de Tauri. No pueden llevar puntos (Tauri 2 solo admite
/// alfanuméricos y `- / : _`); el nombre de contrato con puntos sigue dentro
/// del payload como `type`.
const EVENT_STATE: &str = "session-state-changed";
const EVENT_MESSAGE: &str = "conversation-message-appended";
const EVENT_CONFIRM: &str = "permission-confirmation-required";
const EVENT_HEFESTO: &str = "hefesto-state-changed";

fn emit_all(app: &AppHandle, events: &[atlas_core::OutEvent]) {
    for event in events {
        match event {
            atlas_core::OutEvent::Session(e) => {
                let _ = app.emit(EVENT_STATE, e);
            }
            atlas_core::OutEvent::Message(e) => {
                let _ = app.emit(EVENT_MESSAGE, e);
            }
            atlas_core::OutEvent::Confirm(e) => {
                let _ = app.emit(EVENT_CONFIRM, e);
            }
        }
    }
}

fn poisoned() -> AtlasError {
    AtlasError::new(codes::INTERNAL, "Núcleo no disponible.", true)
}

/// Los comandos de Tauri síncronos corren en el hilo principal: cualquier
/// E/S bloquenate ahí congela teclado y repintado. Toda la red va en el pool.
async fn run_blocking<T, F>(work: F) -> Result<T, AtlasError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AtlasError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| poisoned())?
}

#[derive(Debug, Clone, serde::Serialize)]
struct SendResult {
    #[serde(rename = "requestId")]
    request_id: String,
    #[serde(rename = "sessionId")]
    session_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct DrainResult {
    delivered: usize,
    skipped: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
struct MessageView {
    #[serde(rename = "requestId")]
    request_id: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    role: atlas_core::Role,
    content: String,
    source: Source,
    timestamp: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ActiveSessionView {
    #[serde(rename = "sessionId")]
    session_id: String,
    state: atlas_core::SessionState,
    messages: Vec<MessageView>,
}

/// Drena los eventos nuevos del cerebro, actualiza el cursor y los emite.
fn drain_blocking(
    url: &str,
    cursor: &Arc<Mutex<u64>>,
    app: &AppHandle,
) -> Result<DrainResult, AtlasError> {
    let since = *cursor.lock().map_err(|_| poisoned())?;
    let fetched = get_events(url, since, Duration::from_secs(3))?;
    let max = fetched
        .items
        .iter()
        .map(|(seq, _)| *seq)
        .max()
        .unwrap_or(since);
    if let Ok(mut stored) = cursor.lock() {
        if max > *stored {
            *stored = max;
        }
    }
    let events: Vec<atlas_core::OutEvent> =
        fetched.items.into_iter().map(|(_, event)| event).collect();
    emit_all(app, &events);
    Ok(DrainResult {
        delivered: events.len(),
        skipped: fetched.skipped,
    })
}

/// Envía un mensaje al cerebro y drena de inmediato (eco sin espera).
#[tauri::command]
async fn conversation_send(
    content: String,
    source: Option<String>,
    state: State<'_, CoreState>,
    app: AppHandle,
) -> Result<SendResult, AtlasError> {
    let src = parse_source(source)?;
    let url = state.relay_url.clone();
    let cursor = Arc::clone(&state.relay_cursor);
    run_blocking(move || {
        let (request_id, session_id) = post_send(&url, &content, src, Duration::from_secs(10))?;
        // Drenado inmediato: el input local se representa sin esperar al ciclo.
        let _ = drain_blocking(&url, &cursor, &app);
        Ok(SendResult {
            request_id,
            session_id,
        })
    })
    .await
}

/// Pide al cerebro cancelar una petición (idempotente).
#[tauri::command]
async fn conversation_cancel(
    request_id: String,
    state: State<'_, CoreState>,
    app: AppHandle,
) -> Result<(), AtlasError> {
    let url = state.relay_url.clone();
    let cursor = Arc::clone(&state.relay_cursor);
    run_blocking(move || {
        post_cancel(&url, &request_id, Duration::from_secs(5))?;
        let _ = drain_blocking(&url, &cursor, &app);
        Ok(())
    })
    .await
}

/// Responde a una petición de confirmación (`allow_once` | `deny`).
/// Devuelve si fue aceptada (false también al caducar: idempotente).
#[tauri::command]
async fn permission_respond(
    request_id: String,
    decision: String,
    state: State<'_, CoreState>,
) -> Result<bool, AtlasError> {
    if decision != "allow_once" && decision != "deny" {
        return Err(AtlasError::new(
            codes::BAD_DECISION,
            "Decisión inválida.",
            false,
        ));
    }
    let url = state.relay_url.clone();
    run_blocking(move || {
        atlas_core::post_confirm(&url, &request_id, &decision, Duration::from_secs(5))
    })
    .await
}
/// Arranca el cursor en el borde vivo: lo ya mostrado no se re-juega.
#[tauri::command]
async fn session_get(state: State<'_, CoreState>) -> Result<Option<ActiveSessionView>, AtlasError> {
    let url = state.relay_url.clone();
    let cursor = Arc::clone(&state.relay_cursor);
    run_blocking(move || {
        let (snapshot, current) = get_session(&url, Duration::from_secs(5))?;
        if let Ok(mut stored) = cursor.lock() {
            if current > *stored {
                *stored = current;
            }
        }
        Ok(snapshot.map(|session| ActiveSessionView {
            session_id: session.session_id,
            state: session.state,
            messages: session
                .messages
                .into_iter()
                .map(|m| MessageView {
                    request_id: m.request_id,
                    session_id: m.session_id,
                    role: m.role,
                    content: m.content,
                    source: m.source,
                    timestamp: m.timestamp,
                })
                .collect(),
        }))
    })
    .await
}

/// Drena los eventos nuevos del cerebro (ciclo del frontend).
#[tauri::command]
async fn relay_drain(
    state: State<'_, CoreState>,
    app: AppHandle,
) -> Result<DrainResult, AtlasError> {
    let url = state.relay_url.clone();
    let cursor = Arc::clone(&state.relay_cursor);
    run_blocking(move || drain_blocking(&url, &cursor, &app)).await
}

// ── Hefesto (Fase 4) ──────────────────────────────────────────────────
//
// Bucle nativo del dispositivo, ligado a la vida de la app: un solo hilo
// hace los latidos, sondea y ejecuta el eco. El hilo no lanza procesos; solo
// persiste un marcador de suspensión en el directorio de datos de la app y
// muere con el proceso. Sin credenciales no hay hilo y la UI oculta el indicador.

/// Cadencia del bucle: como mucho un viaje de red por vuelta.
const HEFESTO_TICK: Duration = Duration::from_millis(250);
/// Menor que la lease (15 s): un cuelgue puntual no la agota.
const HEFESTO_TIMEOUT: Duration = Duration::from_secs(5);
const HEFESTO_DEFAULT_URL: &str = "https://atlas-hub.local:8767";
/// Suspensión pendiente de reactivación manual: mientras exista, abrir la app
/// no vuelve a habilitar la ejecución por su cuenta.
const HEFESTO_SUSPEND_MARKER: &str = "hefesto-suspended";

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
struct Hefesto {
    snapshot: Arc<Mutex<Snapshot>>,
    commands: Arc<Mutex<Option<mpsc::Sender<HefestoCommand>>>>,
}

impl Hefesto {
    fn new() -> Self {
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

    fn start(&self, app: AppHandle) {
        let (client, device_id) = match hefesto_transport() {
            Ok(transport) => transport,
            Err(snapshot) => {
                self.store(snapshot);
                return;
            }
        };
        self.store(Snapshot::connecting(&device_id));
        let marker = app
            .path()
            .app_data_dir()
            .ok()
            .map(|dir| dir.join(HEFESTO_SUSPEND_MARKER));
        let snapshot = Arc::clone(&self.snapshot);
        let commands = Arc::clone(&self.commands);
        let spawned = thread::Builder::new()
            .name("hefesto".to_string())
            .spawn(move || {
                hefesto_loop(
                    Runtime::new(client, device_id),
                    Instant::now(),
                    snapshot,
                    commands,
                    app,
                    marker,
                )
            });
        if spawned.is_err() {
            self.store(Snapshot::failed("thread_failed"));
        }
    }
}

/// Credenciales y URL solo por entorno: los ficheros viven fuera del repo.
/// Sin las cuatro variables de identidad, Hefesto queda sin configurar.
fn hefesto_transport() -> std::result::Result<(HefestoClient, OpaqueId), Snapshot> {
    let (Ok(ca_path), Ok(cert_path), Ok(key_path), Ok(device_id)) = (
        std::env::var("ATLAS_HEFESTO_CA"),
        std::env::var("ATLAS_HEFESTO_CERT"),
        std::env::var("ATLAS_HEFESTO_KEY"),
        std::env::var("ATLAS_HEFESTO_DEVICE_ID"),
    ) else {
        return Err(Snapshot::unconfigured());
    };
    let url =
        std::env::var("ATLAS_HEFESTO_URL").unwrap_or_else(|_| HEFESTO_DEFAULT_URL.to_string());
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
enum GateAction {
    None,
    Resume,
    Suspend,
    Activate,
}

/// Traduce una transición de fase en lo que hay que hacer con la intención
/// persistida. Un registro nuevo reactiva solo si el usuario la dejó activa;
/// una suspensión desde `Ready` es remota y exige activación manual.
fn hefesto_gate(previous: Phase, current: Phase, active: bool) -> GateAction {
    match current {
        Phase::Ready if !active => GateAction::Activate,
        Phase::Suspended if previous == Phase::Ready && active => GateAction::Suspend,
        Phase::Suspended if previous != Phase::Suspended && active => GateAction::Resume,
        _ => GateAction::None,
    }
}

/// Lectura conservadora: si el marcador no se puede consultar, se asume
/// suspendido; solo la ausencia clara lo deja activo.
fn hefesto_is_suspended(path: Option<&Path>) -> bool {
    let Some(path) = path else {
        return false;
    };
    match std::fs::metadata(path) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

fn hefesto_write_suspended(path: Option<&Path>) {
    let Some(path) = path else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, b"suspended\n");
}

fn hefesto_clear_suspended(path: Option<&Path>) {
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
        EVENT_HEFESTO,
        HefestoEvent {
            kind: "hefesto.state_changed",
            snapshot: current,
        },
    );
}

#[tauri::command]
fn hefesto_status(hefesto: State<'_, Hefesto>) -> Snapshot {
    hefesto.current()
}

#[tauri::command]
fn hefesto_resume(hefesto: State<'_, Hefesto>) {
    hefesto.send(HefestoCommand::Resume);
}

#[tauri::command]
fn hefesto_abort(hefesto: State<'_, Hefesto>) {
    hefesto.send(HefestoCommand::Abort);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri_plugin_global_shortcut::ShortcutState;

    tauri::Builder::default()
        .manage(CoreState {
            relay_url: default_relay_url(),
            relay_cursor: Arc::new(Mutex::new(0)),
        })
        .manage(Hefesto::new())
        .setup(|app| {
            app.state::<Hefesto>().start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            conversation_send,
            conversation_cancel,
            session_get,
            relay_drain,
            permission_respond,
            hefesto_status,
            hefesto_resume,
            hefesto_abort,
        ])
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts(["command+shift+a"])
                .expect("atajo global no registrable")
                .with_handler(|app, _, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if let Some(window) = app.get_webview_window("main") {
                        let visible = window.is_visible().unwrap_or(false);
                        let focused = window.is_focused().unwrap_or(false);
                        if visible && focused {
                            let _ = window.hide();
                        } else {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(),
        )
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{
        hefesto_clear_suspended, hefesto_gate, hefesto_is_suspended, hefesto_write_suspended,
        GateAction, Phase, EVENT_CONFIRM, EVENT_HEFESTO, EVENT_MESSAGE, EVENT_STATE,
    };

    /// Tauri 2 rechaza cualquier nombre con caracteres fuera de este conjunto,
    /// y el rechazo llega como error de `listen`/`emit`: si un canal lo viola,
    /// el frontend se queda sin eventos en silencio.
    fn canal_valido(name: &str) -> bool {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '/' | ':' | '_'))
    }

    #[test]
    fn canales_ipc_aceptados_por_tauri() {
        for channel in [EVENT_STATE, EVENT_MESSAGE, EVENT_CONFIRM, EVENT_HEFESTO] {
            assert!(canal_valido(channel), "canal IPC inválido: {channel}");
        }
    }

    #[test]
    fn un_registro_nuevo_reactiva_solo_si_la_dejaste_activa() {
        for previous in [Phase::Connecting, Phase::Offline] {
            assert_eq!(
                hefesto_gate(previous, Phase::Suspended, true),
                GateAction::Resume
            );
            assert_eq!(
                hefesto_gate(previous, Phase::Suspended, false),
                GateAction::None
            );
        }
    }

    #[test]
    fn la_suspension_exige_reactivacion_manual() {
        // Remota o local: nunca se reactiva sola mientras siga suspendida.
        assert_eq!(
            hefesto_gate(Phase::Ready, Phase::Suspended, true),
            GateAction::Suspend
        );
        assert_eq!(
            hefesto_gate(Phase::Ready, Phase::Suspended, false),
            GateAction::None
        );
        assert_eq!(
            hefesto_gate(Phase::Suspended, Phase::Suspended, true),
            GateAction::None
        );
        assert_eq!(
            hefesto_gate(Phase::Suspended, Phase::Ready, false),
            GateAction::Activate
        );
    }

    #[test]
    fn el_marcador_de_suspension_sobrevive_hasta_limpiarlo() {
        let path = std::env::temp_dir().join(format!(
            "atlas-hefesto-suspended-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        ));
        assert!(!hefesto_is_suspended(Some(&path)));
        hefesto_write_suspended(Some(&path));
        assert!(hefesto_is_suspended(Some(&path)));
        hefesto_clear_suspended(Some(&path));
        assert!(!hefesto_is_suspended(Some(&path)));
    }
}
