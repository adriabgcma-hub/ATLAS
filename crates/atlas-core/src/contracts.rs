//! Contratos compartidos del núcleo (Fase 2).
//!
//! Espejo de `docs/contracts.md` y `src/types/contracts.ts`: cualquier cambio
//! debe hacerse en los tres sitios a la vez. Los identificadores se generan en
//! el núcleo y nunca contienen secretos ni datos personales.

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Límite de contenido aceptado por mensaje (ver `contracts.md`).
pub const MAX_CONTENT_LEN: usize = 4000;

/// Códigos de error estables expuestos al frontend (seguros para mostrar).
pub mod codes {
    pub const EMPTY_MESSAGE: &str = "empty_message";
    pub const MESSAGE_TOO_LONG: &str = "message_too_long";
    pub const UNKNOWN_REQUEST: &str = "unknown_request";
    pub const INVALID_SOURCE: &str = "invalid_source";
    pub const RELAY_UNREACHABLE: &str = "relay_unreachable";
    pub const RELAY_INVALID: &str = "relay_invalid";
    pub const INTERNAL: &str = "internal_error";
}

/// Estado de una sesión. Serializa exactamente como en `contracts.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Thinking,
    AwaitingConfirmation,
    Executing,
    Responding,
    Cancelled,
    Completed,
    Failed,
}

/// Canal de entrada. Ambos se tratan igual; solo cambia el origen registrado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Local,
    Telegram,
}

/// Rol de un mensaje dentro de la conversación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// Nombre de evento tal cual viaja por IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EventType {
    #[serde(rename = "session.state_changed")]
    SessionStateChanged,
    #[serde(rename = "conversation.message_appended")]
    MessageAppended,
}

/// Identificadores de correlación. Se generan en el núcleo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Correlation {
    #[serde(rename = "requestId")]
    pub request_id: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub timestamp: String,
}

impl Correlation {
    pub fn new(session_id: &str) -> Self {
        Self {
            request_id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            timestamp: now_iso8601(),
        }
    }
}

/// Instante actual en ISO-8601 UTC.
pub fn now_iso8601() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Evento `session.state_changed`. El frontend solo lo representa.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionEvent {
    #[serde(flatten)]
    pub correlation: Correlation,
    #[serde(rename = "type")]
    pub kind: EventType,
    pub state: SessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl SessionEvent {
    pub fn new(session_id: &str, request_id: &str, state: SessionState) -> Self {
        Self {
            correlation: Correlation {
                request_id: request_id.to_string(),
                session_id: session_id.to_string(),
                timestamp: now_iso8601(),
            },
            kind: EventType::SessionStateChanged,
            state,
            message: None,
        }
    }
}

/// Evento `conversation.message_appended`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageAppended {
    #[serde(flatten)]
    pub correlation: Correlation,
    #[serde(rename = "type")]
    pub kind: EventType,
    pub role: Role,
    pub content: String,
    pub source: Source,
}

/// Error seguro para mostrar en el frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtlasError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl AtlasError {
    pub fn new(code: &str, message: &str, retryable: bool) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
            retryable,
            request_id: None,
        }
    }

    pub fn with_request(mut self, request_id: &str) -> Self {
        self.request_id = Some(request_id.to_string());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_event_serializa_con_nombres_de_contrato() {
        let event = SessionEvent::new("ses-1", "req-1", SessionState::Thinking);
        let json = serde_json::to_value(&event).expect("serializa");
        assert_eq!(json["type"], "session.state_changed");
        assert_eq!(json["state"], "thinking");
        assert_eq!(json["requestId"], "req-1");
        assert_eq!(json["sessionId"], "ses-1");
    }

    #[test]
    fn source_rechaza_origenes_desconocidos() {
        let parsed: Result<Source, _> = serde_json::from_str("\"sms\"");
        assert!(parsed.is_err());
    }

    #[test]
    fn correlation_genera_ids_unicos() {
        let a = Correlation::new("ses-1");
        let b = Correlation::new("ses-1");
        assert_ne!(a.request_id, b.request_id);
    }
}
