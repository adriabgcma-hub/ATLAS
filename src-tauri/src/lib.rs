//! Capa Tauri de Atlas (Fase 3): ventana, atajo global y comandos IPC.
//!
//! Fina por diseño: el cerebro (sesiones, Groq, Telegram) vive en el relay
//! Python. Esta capa transporta peticiones y eventos, sin estado propio.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_core::{codes, get_events, get_session, post_cancel, post_send, AtlasError, Source};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

/// Puente real de control de apps (`NSWorkspace`), solo en macOS: AppKit no
/// está disponible en el resto de plataformas.
#[cfg(target_os = "macos")]
mod hefesto_apps;

/// Puente real de teclado (`CGEvent` + permiso TCC), solo en macOS.
#[cfg(target_os = "macos")]
mod hefesto_keyboard;

/// Puente real de ventanas (`AXUIElement` + AppKit), solo en macOS.
#[cfg(target_os = "macos")]
mod hefesto_windows;

/// Consultas del sistema por adaptadores fijos, solo en macOS.
#[cfg(target_os = "macos")]
mod hefesto_system;

/// Terminal general con argv y supervisión, solo en macOS.
#[cfg(target_os = "macos")]
mod hefesto_terminal;

/// Captura de pantalla con OCR local, solo en macOS.
#[cfg(target_os = "macos")]
mod hefesto_capture;

/// Ventana de tablas y exportación CSV.
mod table;

/// Runtime nativo del dispositivo Hefesto (estado, bucle y comandos IPC).
mod hefesto_runtime;

use hefesto_runtime::{hefesto_abort, hefesto_resume, hefesto_status, Hefesto};

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
    #[serde(rename = "agent", skip_serializing_if = "Option::is_none")]
    agent: Option<atlas_core::Agent>,
    #[serde(rename = "taskId", skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
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
                    agent: m.agent,
                    task_id: m.task_id.clone(),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VisibilityAction {
    Show,
    Hide,
}

/// El atajo alterna: oculta solo si la ventana está visible y enfocada;
/// en cualquier otro caso la trae al frente.
fn next_visibility(visible: bool, focused: bool) -> VisibilityAction {
    if visible && focused {
        VisibilityAction::Hide
    } else {
        VisibilityAction::Show
    }
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
        .manage(table::TableState::new())
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
            table::open_table_window,
            table::table_payload,
            table::export_table_csv,
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
                        match next_visibility(
                            window.is_visible().unwrap_or(false),
                            window.is_focused().unwrap_or(false),
                        ) {
                            VisibilityAction::Hide => {
                                let _ = window.hide();
                            }
                            VisibilityAction::Show => {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(),
        )
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // La ventana principal queda oculta; las de tabla se cierran
                // de verdad y su payload se libera en `Destroyed`.
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            if let WindowEvent::Destroyed = event {
                let label = window.label();
                if label.starts_with("table-") {
                    window
                        .app_handle()
                        .state::<table::TableState>()
                        .remove(label);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{EVENT_CONFIRM, EVENT_HEFESTO, EVENT_MESSAGE, EVENT_STATE};
    use crate::hefesto_runtime::{
        hefesto_clear_suspended, hefesto_gate, hefesto_is_suspended, hefesto_write_suspended,
        GateAction,
    };
    use atlas_mac::Phase;

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

    #[test]
    fn el_atajo_oculta_solo_si_esta_visible_y_en_foco() {
        use super::{next_visibility, VisibilityAction};
        assert!(matches!(
            next_visibility(true, true),
            VisibilityAction::Hide
        ));
        assert!(matches!(
            next_visibility(true, false),
            VisibilityAction::Show
        ));
        assert!(matches!(
            next_visibility(false, true),
            VisibilityAction::Show
        ));
        assert!(matches!(
            next_visibility(false, false),
            VisibilityAction::Show
        ));
    }

    fn config_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("atlas-config-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("directorio temporal");
        dir
    }

    #[test]
    fn el_entorno_gana_al_fichero_de_configuracion() {
        use crate::hefesto_runtime::config_value;
        let dir = config_dir("entorno");
        std::fs::write(dir.join("device-id"), "desde-fichero").expect("fichero");
        assert_eq!(
            config_value(Some("desde-entorno".to_owned()), Some(&dir), "device-id").as_deref(),
            Some("desde-entorno")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sin_entorno_se_lee_el_fichero_recortado() {
        use crate::hefesto_runtime::config_value;
        let dir = config_dir("fichero");
        std::fs::write(dir.join("device-id"), "  mac-one\n").expect("fichero");
        assert_eq!(
            config_value(None, Some(&dir), "device-id").as_deref(),
            Some("mac-one")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn un_entorno_vacio_no_tapa_el_fichero() {
        use crate::hefesto_runtime::config_value;
        let dir = config_dir("vacio");
        std::fs::write(dir.join("device-id"), "mac-one").expect("fichero");
        assert_eq!(
            config_value(Some("  ".to_owned()), Some(&dir), "device-id").as_deref(),
            Some("mac-one")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sin_fichero_ni_entorno_no_hay_configuracion() {
        use crate::hefesto_runtime::config_value;
        let dir = config_dir("ausente");
        assert_eq!(config_value(None, Some(&dir), "device-id"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn el_entorno_de_ruta_gana_al_fichero() {
        use crate::hefesto_runtime::config_path;
        let dir = config_dir("ruta-entorno");
        std::fs::write(dir.join("ca.pem"), b"certificado").expect("fichero");
        assert_eq!(
            config_path(Some("/tmp/ca.pem".to_owned()), Some(&dir), "ca.pem").as_deref(),
            Some("/tmp/ca.pem")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sin_entorno_la_ruta_apunta_al_fichero() {
        use crate::hefesto_runtime::config_path;
        let dir = config_dir("ruta-fichero");
        let expected = dir.join("ca.pem");
        std::fs::write(&expected, b"certificado").expect("fichero");
        assert_eq!(
            config_path(None, Some(&dir), "ca.pem").as_deref(),
            Some(expected.to_string_lossy().as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sin_fichero_no_hay_ruta() {
        use crate::hefesto_runtime::config_path;
        let dir = config_dir("ruta-ausente");
        assert_eq!(config_path(None, Some(&dir), "ca.pem"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn la_clave_privada_exige_permisos_sin_grupo_ni_otros() {
        use crate::hefesto_runtime::private_file;
        use std::os::unix::fs::PermissionsExt;
        let dir = config_dir("permisos");
        let path = dir.join("device-key.pem");
        std::fs::write(&path, b"clave").expect("clave");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("permisos");
        assert!(private_file(&path));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("permisos");
        assert!(!private_file(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
