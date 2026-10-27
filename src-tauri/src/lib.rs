//! Capa Tauri de Atlas (Fase 3): ventana, atajo global y comandos IPC.
//!
//! Fina por diseño: el cerebro (sesiones, Groq, Telegram) vive en el relay
//! Python. Esta capa transporta peticiones y eventos, sin estado propio.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_core::{AtlasError, Source, codes, get_events, get_session, post_cancel, post_send};
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri_plugin_global_shortcut::ShortcutState;

    tauri::Builder::default()
        .manage(CoreState {
            relay_url: default_relay_url(),
            relay_cursor: Arc::new(Mutex::new(0)),
        })
        .invoke_handler(tauri::generate_handler![
            conversation_send,
            conversation_cancel,
            session_get,
            relay_drain,
            permission_respond,
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
    use super::{EVENT_CONFIRM, EVENT_MESSAGE, EVENT_STATE};

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
        for channel in [EVENT_STATE, EVENT_MESSAGE, EVENT_CONFIRM] {
            assert!(canal_valido(channel), "canal IPC inválido: {channel}");
        }
    }
}
