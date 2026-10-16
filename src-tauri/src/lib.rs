//! Capa Tauri de Atlas (Fase 3): ventana, atajo global y comandos IPC.
//!
//! Fina por diseño: el cerebro (sesiones, Groq, Telegram) vive en el relay
//! Python. Esta capa transporta peticiones y eventos, sin estado propio.

use std::sync::Mutex;
use std::time::Duration;

use atlas_core::{AtlasError, Source, codes, get_events, get_session, post_cancel, post_send};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

/// URL del cerebro en la red local. No es un secreto.
fn default_relay_url() -> String {
    std::env::var("ATLAS_RELAY_URL").unwrap_or_else(|_| "http://127.0.0.1:8765".to_string())
}

struct CoreState {
    relay_url: String,
    relay_cursor: Mutex<u64>,
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

fn emit_all(app: &AppHandle, events: &[atlas_core::OutEvent]) {
    for event in events {
        match event {
            atlas_core::OutEvent::Session(e) => {
                let _ = app.emit("session.state_changed", e);
            }
            atlas_core::OutEvent::Message(e) => {
                let _ = app.emit("conversation.message_appended", e);
            }
        }
    }
}

fn poisoned() -> AtlasError {
    AtlasError::new(codes::INTERNAL, "Núcleo no disponible.", true)
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
fn drain_events(state: &State<CoreState>, app: &AppHandle) -> Result<DrainResult, AtlasError> {
    let (url, cursor) = {
        let cursor = state.relay_cursor.lock().map_err(|_| poisoned())?;
        (state.relay_url.clone(), *cursor)
    };
    let fetched = get_events(&url, cursor, Duration::from_secs(3))?;
    let max = fetched
        .items
        .iter()
        .map(|(seq, _)| *seq)
        .max()
        .unwrap_or(cursor);
    if let Ok(mut stored) = state.relay_cursor.lock() {
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
fn conversation_send(
    content: String,
    source: Option<String>,
    state: State<CoreState>,
    app: AppHandle,
) -> Result<SendResult, AtlasError> {
    let src = parse_source(source)?;
    let url = state.relay_url.clone();
    let (request_id, session_id) =
        post_send(&url, &content, src, Duration::from_secs(10))?;
    // Drenado inmediato: el input local se representa sin esperar al ciclo.
    let _ = drain_events(&state, &app);
    Ok(SendResult {
        request_id,
        session_id,
    })
}

/// Pide al cerebro cancelar una petición (idempotente).
#[tauri::command]
fn conversation_cancel(
    request_id: String,
    state: State<CoreState>,
    app: AppHandle,
) -> Result<(), AtlasError> {
    let url = state.relay_url.clone();
    post_cancel(&url, &request_id, Duration::from_secs(5))?;
    let _ = drain_events(&state, &app);
    Ok(())
}

/// Instantánea de la sesión activa del cerebro (para hidratar al abrir).
/// Arranca el cursor en el borde vivo: lo ya mostrado no se re-juega.
#[tauri::command]
fn session_get(state: State<CoreState>) -> Result<Option<ActiveSessionView>, AtlasError> {
    let url = state.relay_url.clone();
    let (snapshot, cursor) = get_session(&url, Duration::from_secs(5))?;
    if let Ok(mut stored) = state.relay_cursor.lock() {
        if cursor > *stored {
            *stored = cursor;
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
}

/// Drena los eventos nuevos del cerebro (ciclo del frontend).
#[tauri::command]
fn relay_drain(state: State<CoreState>, app: AppHandle) -> Result<DrainResult, AtlasError> {
    drain_events(&state, &app)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri_plugin_global_shortcut::ShortcutState;

    tauri::Builder::default()
        .manage(CoreState {
            relay_url: default_relay_url(),
            relay_cursor: Mutex::new(0),
        })
        .invoke_handler(tauri::generate_handler![
            conversation_send,
            conversation_cancel,
            session_get,
            relay_drain,
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
