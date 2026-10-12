//! Sesiones y operaciones de Atlas (Fase 2).
//!
//! `SessionManager` es el dueño único de las sesiones y de las transiciones de
//! estado. Puro y síncrono: la espera del stub vive en la capa Tauri, que llama
//! a `complete_stub` pasado el retardo. Sin persistencia (eso es Fase 4).

use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

use super::contracts::{
    AtlasError, Correlation, EventType, MessageAppended, Role, SessionEvent, SessionState,
    Source, codes, now_iso8601,
};
use super::relay::RelayMessage;

/// Retardo del stub local antes de responder. Lo sustituye el proveedor en Fase 3.
pub const STUB_DELAY: Duration = Duration::from_millis(700);
/// Tiempo máximo de una operación en curso antes de marcarla fallida.
pub const OP_TIMEOUT: Duration = Duration::from_secs(30);

/// Mensaje guardado en una sesión.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub request_id: String,
    pub session_id: String,
    pub role: Role,
    pub content: String,
    pub source: Source,
    pub timestamp: String,
}

/// Operación en curso (un `send` aún no terminado).
#[derive(Debug, Clone)]
struct PendingOp {
    request_id: String,
    session_id: String,
    state: SessionState,
    deadline: Instant,
}

/// Sesión de conversación.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub state: SessionState,
    pub messages: Vec<ChatMessage>,
    pub created_at: String,
    pub updated_at: String,
}

/// Eventos listos para emitir por IPC, en orden.
#[derive(Debug, Clone)]
pub enum OutEvent {
    Session(SessionEvent),
    Message(MessageAppended),
}

/// Resultado de `send`: identificadores más eventos inmediatos.
#[derive(Debug, Clone)]
pub struct SendOutcome {
    pub request_id: String,
    pub session_id: String,
    pub events: Vec<OutEvent>,
}

/// Resultado de drenar el relay.
#[derive(Debug, Clone)]
pub struct DrainOutcome {
    pub delivered: usize,
    pub skipped: usize,
    pub outcomes: Vec<SendOutcome>,
}

/// Respuesta local de Fase 2. La Fase 3 la sustituye por un proveedor real
/// detrás de este mismo interfaz (sin tocar sesiones ni IPC).
pub trait Responder {
    fn respond(&self, content: &str) -> String;
}

/// Stub local: ejercita estados, cancelación y timeouts sin ninguna IA.
#[derive(Debug, Clone, Copy)]
pub struct StubResponder;

impl Responder for StubResponder {
    fn respond(&self, content: &str) -> String {
        format!("(stub local · Fase 2) Recibido: {content}")
    }
}

fn is_terminal(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::Idle
            | SessionState::Cancelled
            | SessionState::Completed
            | SessionState::Failed
    )
}

/// Transiciones permitidas entre estados de operación.
pub fn is_transition_allowed(from: SessionState, to: SessionState) -> bool {
    match from {
        SessionState::Thinking => matches!(
            to,
            SessionState::Responding | SessionState::Cancelled | SessionState::Failed
        ),
        SessionState::Responding => matches!(
            to,
            SessionState::Completed | SessionState::Cancelled | SessionState::Failed
        ),
        _ => false,
    }
}

fn validate_content(content: &str) -> Result<String, AtlasError> {
    let text = content.trim().to_string();
    if text.is_empty() {
        return Err(AtlasError::new(
            codes::EMPTY_MESSAGE,
            "El mensaje está vacío.",
            false,
        ));
    }
    if text.chars().count() > super::contracts::MAX_CONTENT_LEN {
        return Err(AtlasError::new(
            codes::MESSAGE_TOO_LONG,
            "El mensaje supera el límite.",
            false,
        ));
    }
    Ok(text)
}

/// Dueño único de sesiones y transiciones de estado.
#[derive(Debug)]
pub struct SessionManager {
    sessions: HashMap<String, Session>,
    active_session_id: Option<String>,
    pending: HashMap<String, PendingOp>,
    op_timeout: Duration,
    responder: StubResponder,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            active_session_id: None,
            pending: HashMap::new(),
            op_timeout: OP_TIMEOUT,
            responder: StubResponder,
        }
    }

    /// Sesión activa (vista de solo lectura, para `session_get`).
    pub fn active(&self) -> Option<&Session> {
        self.active_session_id
            .as_ref()
            .and_then(|id| self.sessions.get(id))
    }

    fn active_mut(&mut self) -> &mut Session {
        if self.active_session_id.is_none() {
            let id = Uuid::new_v4().to_string();
            let now = now_iso8601();
            self.sessions.insert(
                id.clone(),
                Session {
                    id: id.clone(),
                    state: SessionState::Idle,
                    messages: Vec::new(),
                    created_at: now.clone(),
                    updated_at: now,
                },
            );
            self.active_session_id = Some(id);
        }
        let id = self
            .active_session_id
            .clone()
            .expect("sesión activa recién creada");
        self.sessions.get_mut(&id).expect("sesión activa existe")
    }

    fn emit_state(
        session: &Session,
        request_id: &str,
        state: SessionState,
        message: Option<String>,
    ) -> OutEvent {
        let mut event = SessionEvent::new(&session.id, request_id, state);
        event.message = message;
        OutEvent::Session(event)
    }

    /// Registra un mensaje entrante (canal local o relay) y abre la operación.
    pub fn send(&mut self, content: &str, source: Source) -> Result<SendOutcome, AtlasError> {
        let text = validate_content(content)?;
        let correlation = Correlation::new("");
        let request_id = correlation.request_id.clone();

        let session = self.active_mut();
        session.state = SessionState::Thinking;
        session.updated_at = now_iso8601();
        let session_id = session.id.clone();
        session.messages.push(ChatMessage {
            request_id: request_id.clone(),
            session_id: session_id.clone(),
            role: Role::User,
            content: text.clone(),
            source,
            timestamp: correlation.timestamp.clone(),
        });

        self.pending.insert(
            request_id.clone(),
            PendingOp {
                request_id: request_id.clone(),
                session_id: session_id.clone(),
                state: SessionState::Thinking,
                deadline: Instant::now() + self.op_timeout,
            },
        );

        let message_event = MessageAppended {
            correlation: Correlation {
                request_id: request_id.clone(),
                session_id: session_id.clone(),
                timestamp: correlation.timestamp,
            },
            kind: EventType::MessageAppended,
            role: Role::User,
            content: text,
            source,
        };

        Ok(SendOutcome {
            request_id: request_id.clone(),
            session_id: session_id.clone(),
            events: vec![
                Self::emit_state(
                    self.sessions.get(&session_id).expect("sesión existe"),
                    &request_id,
                    SessionState::Thinking,
                    None,
                ),
                OutEvent::Message(message_event),
            ],
        })
    }

    /// Completa la operación con la respuesta del stub. Idempotente: si la
    /// operación se canceló, ya terminó o nunca existió, no emite nada.
    /// (La tarea diferida de la capa Tauri llama aquí siempre; el camino
    /// normal tras un `cancel` no debe producir errores.)
    pub fn complete_stub(&mut self, request_id: &str) -> Result<Vec<OutEvent>, AtlasError> {
        let op = match self.pending.get(request_id).cloned() {
            Some(op) => op,
            None => return Ok(Vec::new()),
        };
        if is_terminal(op.state) {
            return Ok(Vec::new());
        }
        if !is_transition_allowed(op.state, SessionState::Responding) {
            return Ok(Vec::new());
        }

        let session = self
            .sessions
            .get_mut(&op.session_id)
            .expect("la sesión de una operación existe");
        let reply = self.responder.respond(
            &session
                .messages
                .iter()
                .rev()
                .find(|m| m.request_id == request_id)
                .map(|m| m.content.clone())
                .unwrap_or_default(),
        );
        let correlation = Correlation::new(&op.session_id);
        session.state = SessionState::Completed;
        session.updated_at = correlation.timestamp.clone();
        session.messages.push(ChatMessage {
            request_id: correlation.request_id.clone(),
            session_id: op.session_id.clone(),
            role: Role::Assistant,
            content: reply.clone(),
            source: Source::Local,
            timestamp: correlation.timestamp.clone(),
        });
        if let Some(stored) = self.pending.get_mut(request_id) {
            stored.state = SessionState::Completed;
        }
        self.pending.remove(request_id);

        let session_snapshot = self.sessions.get(&op.session_id).expect("sesión existe").clone();
        Ok(vec![
            Self::emit_state(&session_snapshot, request_id, SessionState::Responding, None),
            OutEvent::Message(MessageAppended {
                correlation,
                kind: EventType::MessageAppended,
                role: Role::Assistant,
                content: reply,
                source: Source::Local,
            }),
            Self::emit_state(&session_snapshot, request_id, SessionState::Completed, None),
        ])
    }

    /// Cancela una operación en curso. Idempotente ante terminales.
    pub fn cancel(&mut self, request_id: &str) -> Result<Vec<OutEvent>, AtlasError> {
        let op = self
            .pending
            .get(request_id)
            .cloned()
            .ok_or_else(|| AtlasError::new(codes::UNKNOWN_REQUEST, "Petición desconocida.", false))?;
        if is_terminal(op.state) {
            return Ok(Vec::new());
        }
        let session = self
            .sessions
            .get_mut(&op.session_id)
            .expect("la sesión de una operación existe");
        session.state = SessionState::Cancelled;
        session.updated_at = now_iso8601();
        self.pending.remove(request_id);
        let snapshot = session.clone();
        Ok(vec![Self::emit_state(
            &snapshot,
            request_id,
            SessionState::Cancelled,
            None,
        )])
    }

    /// Marca como fallidas las operaciones que superaron su plazo.
    pub fn sweep_timeouts(&mut self) -> Vec<OutEvent> {
        let now = Instant::now();
        let expired: Vec<PendingOp> = self
            .pending
            .values()
            .filter(|op| !is_terminal(op.state) && op.deadline <= now)
            .cloned()
            .collect();
        let mut events = Vec::new();
        for op in expired {
            if let Some(session) = self.sessions.get_mut(&op.session_id) {
                session.state = SessionState::Failed;
                session.updated_at = now_iso8601();
                let snapshot = session.clone();
                events.push(Self::emit_state(
                    &snapshot,
                    &op.request_id,
                    SessionState::Failed,
                    Some("timeout".to_string()),
                ));
            }
            self.pending.remove(&op.request_id);
        }
        events
    }

    /// Ingiere mensajes del relay en orden (FIFO). Los inválidos se cuentan
    /// como omitidos sin abortar el resto.
    pub fn ingest(&mut self, items: Vec<RelayMessage>) -> DrainOutcome {
        let mut delivered = 0;
        let mut skipped = 0;
        let mut outcomes = Vec::new();
        for item in items {
            match self.send(&item.content, item.source) {
                Ok(outcome) => {
                    delivered += 1;
                    outcomes.push(outcome);
                }
                Err(_) => {
                    skipped += 1;
                }
            }
        }
        DrainOutcome {
            delivered,
            skipped,
            outcomes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_of(events: &[OutEvent]) -> Vec<SessionState> {
        events
            .iter()
            .filter_map(|e| match e {
                OutEvent::Session(s) => Some(s.state),
                OutEvent::Message(_) => None,
            })
            .collect()
    }

    #[test]
    fn send_emite_thinking_y_mensaje_de_usuario() {
        let mut mgr = SessionManager::new();
        let out = mgr.send("hola", Source::Local).expect("send ok");
        assert_eq!(state_of(&out.events), vec![SessionState::Thinking]);
        assert!(matches!(out.events[1], OutEvent::Message(_)));
        assert_eq!(mgr.active().expect("sesión").state, SessionState::Thinking);
    }

    #[test]
    fn send_rechaza_vacio_y_demasiado_largo() {
        let mut mgr = SessionManager::new();
        let err = mgr.send("   ", Source::Local).expect_err("vacío");
        assert_eq!(err.code, codes::EMPTY_MESSAGE);
        assert!(!err.retryable);
        let long = "x".repeat(super::super::contracts::MAX_CONTENT_LEN + 1);
        let err = mgr.send(&long, Source::Telegram).expect_err("largo");
        assert_eq!(err.code, codes::MESSAGE_TOO_LONG);
    }

    #[test]
    fn flujo_completo_con_stub() {
        let mut mgr = SessionManager::new();
        let out = mgr.send("hola", Source::Telegram).expect("send ok");
        let events = mgr.complete_stub(&out.request_id).expect("stub ok");
        assert_eq!(
            state_of(&events),
            vec![SessionState::Responding, SessionState::Completed]
        );
        let session = mgr.active().expect("sesión");
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[1].role, Role::Assistant);
    }

    #[test]
    fn cancelar_suprime_la_respuesta_del_stub() {
        let mut mgr = SessionManager::new();
        let out = mgr.send("hola", Source::Local).expect("send ok");
        let cancel_events = mgr.cancel(&out.request_id).expect("cancel ok");
        assert_eq!(state_of(&cancel_events), vec![SessionState::Cancelled]);
        let after = mgr.complete_stub(&out.request_id).expect("stub tras cancel");
        assert!(after.is_empty());
    }

    #[test]
    fn cancelar_peticion_desconocida_falla() {
        let mut mgr = SessionManager::new();
        let err = mgr.cancel("req-inexistente").expect_err("desconocida");
        assert_eq!(err.code, codes::UNKNOWN_REQUEST);
    }

    #[test]
    fn completar_dos_veces_es_idempotente() {
        let mut mgr = SessionManager::new();
        let out = mgr.send("hola", Source::Local).expect("send ok");
        mgr.complete_stub(&out.request_id).expect("primera");
        let second = mgr.complete_stub(&out.request_id).expect("segunda");
        assert!(second.is_empty());
    }

    #[test]
    fn sweep_marca_timeout_como_failed() {
        let mut mgr = SessionManager::new();
        mgr.op_timeout = Duration::from_millis(1);
        let out = mgr.send("hola", Source::Local).expect("send ok");
        std::thread::sleep(Duration::from_millis(5));
        let events = mgr.sweep_timeouts();
        assert_eq!(state_of(&events), vec![SessionState::Failed]);
        assert_eq!(mgr.active().expect("sesión").state, SessionState::Failed);
        let _ = out;
    }

    #[test]
    fn transiciones_validas_e_invalidas() {
        assert!(is_transition_allowed(
            SessionState::Thinking,
            SessionState::Responding
        ));
        assert!(is_transition_allowed(
            SessionState::Thinking,
            SessionState::Cancelled
        ));
        assert!(!is_transition_allowed(
            SessionState::Thinking,
            SessionState::Completed
        ));
        assert!(!is_transition_allowed(
            SessionState::Completed,
            SessionState::Thinking
        ));
        assert!(!is_transition_allowed(SessionState::Idle, SessionState::Failed));
    }
}
