//! Capa Tauri de Atlas (Fase 2): ventana, atajo global y comandos IPC.
//!
//! Fina por diseño: toda la lógica de sesiones vive en `atlas-core`.
//! Esta capa valida el borde, emite eventos y programa el stub diferido.

use std::sync::Mutex;
use std::time::Duration;

use atlas_core::{
    AtlasError, OutEvent, SessionManager, Source, codes, fetch_messages, STUB_DELAY,
};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

/// URL del relay Python en la red local. No es un secreto.
fn default_relay_url() -> String {
    std::env::var("ATLAS_RELAY_URL").unwrap_or_else(|_| "http://127.0.0.1:8765".to_string())
}

struct CoreState {
    manager: Mutex<SessionManager>,
    relay_url: String,
    relay_cursor: Mutex<u64>,
}

fn parse_source(raw: Option<String>) -> Result<Source, AtlasError> {
    match raw.as_deref() {
        None | Some("local") => Ok(Source::Local),
        Some("telegram") => Ok(Source::Telegram),
        Some(_) => Err(AtlasError::new(
            codes::INVALID_SOURCE,
            "Origen desconocido.",
            false,
        )),
    }
}

fn emit_all(app: &AppHandle, events: &[OutEvent]) {
    for event in events {
        match event {
            OutEvent::Session(e) => {
                let _ = app.emit("session.state_changed", e);
            }
            OutEvent::Message(e) => {
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

/// Barre timeouts pendientes y devuelve sus eventos para emitir.
fn take_sweep_events(state: &State<CoreState>) -> Result<Vec<OutEvent>, AtlasError> {
    let mut manager = state.manager.lock().map_err(|_| poisoned())?;
    Ok(manager.sweep_timeouts())
}

/// Registra un mensaje (canal local o Telegram) y abre la operación.
#[tauri::command]
fn conversation_send(
    content: String,
    source: Option<String>,
    state: State<CoreState>,
    app: AppHandle,
) -> Result<SendResult, AtlasError> {
    let src = parse_source(source)?;
    emit_all(&app, &take_sweep_events(&state)?);

    let outcome = {
        let mut manager = state.manager.lock().map_err(|_| poisoned())?;
        manager.send(&content, src)?
    };
    emit_all(&app, &outcome.events);

    // Respuesta diferida del stub (Fase 2). La cancelación la suprime porque
    // `complete_stub` es idempotente ante operaciones terminadas.
    let delayed_app = app.clone();
    let request_id = outcome.request_id.clone();
    std::thread::spawn(move || {
        std::thread::sleep(STUB_DELAY);
        let events = delayed_app
            .state::<CoreState>()
            .manager
            .lock()
            .map(|mut manager| manager.complete_stub(&request_id))
            .unwrap_or(Ok(Vec::new()));
        if let Ok(events) = events {
            emit_all(&delayed_app, &events);
        }
    });

    Ok(SendResult {
        request_id: outcome.request_id,
        session_id: outcome.session_id,
    })
}

/// Cancela una operación en curso. Idempotente ante terminales.
#[tauri::command]
fn conversation_cancel(
    request_id: String,
    state: State<CoreState>,
    app: AppHandle,
) -> Result<(), AtlasError> {
    emit_all(&app, &take_sweep_events(&state)?);
    let events = {
        let mut manager = state.manager.lock().map_err(|_| poisoned())?;
        manager.cancel(&request_id)?
    };
    emit_all(&app, &events);
    Ok(())
}

/// Devuelve la sesión activa, si existe (para hidratar el frontend al abrir).
#[tauri::command]
fn session_get(state: State<CoreState>) -> Result<Option<ActiveSessionView>, AtlasError> {
    let manager = state.manager.lock().map_err(|_| poisoned())?;
    Ok(manager.active().map(|session| ActiveSessionView {
        session_id: session.id.clone(),
        state: session.state,
        messages: session
            .messages
            .iter()
            .map(|m| MessageView {
                request_id: m.request_id.clone(),
                session_id: m.session_id.clone(),
                role: m.role,
                content: m.content.clone(),
                source: m.source,
                timestamp: m.timestamp.clone(),
            })
            .collect(),
    }))
}

/// Drena el relay (polling) e ingiere sus mensajes como canal `telegram`.
#[tauri::command]
fn relay_drain(state: State<CoreState>, app: AppHandle) -> Result<DrainResult, AtlasError> {
    emit_all(&app, &take_sweep_events(&state)?);

    let (url, cursor) = {
        let cursor = state.relay_cursor.lock().map_err(|_| poisoned())?;
        (state.relay_url.clone(), *cursor)
    };
    let fetched = fetch_messages(&url, cursor, Duration::from_secs(3))?;

    let outcome = {
        let mut manager = state.manager.lock().map_err(|_| poisoned())?;
        manager.ingest(fetched.messages.clone())
    };
    let max_seq = fetched
        .messages
        .iter()
        .map(|m| m.seq)
        .max()
        .unwrap_or(cursor);
    if let Ok(mut stored) = state.relay_cursor.lock() {
        if max_seq > *stored {
            *stored = max_seq;
        }
    }

    let mut events = Vec::new();
    for item in &outcome.outcomes {
        events.extend(item.events.clone());
    }
    emit_all(&app, &events);

    Ok(DrainResult {
        delivered: outcome.delivered,
        skipped: outcome.skipped + fetched.skipped,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri_plugin_global_shortcut::ShortcutState;

    tauri::Builder::default()
        .manage(CoreState {
            manager: Mutex::new(SessionManager::new()),
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
