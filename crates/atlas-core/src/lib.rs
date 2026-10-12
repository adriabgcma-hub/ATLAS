//! Núcleo de sesiones de Atlas (Fase 2).
//!
//! Puro y sin Tauri: contratos, sesiones y cliente del relay. La capa
//! `src-tauri` lo envuelve en comandos IPC y eventos.

pub mod contracts;
pub mod relay;
pub mod session;

pub use contracts::{
    AtlasError, Correlation, EventType, MessageAppended, Role, SessionEvent, SessionState,
    Source, codes, now_iso8601, MAX_CONTENT_LEN,
};
pub use relay::{FetchResult, RelayMessage, fetch_messages};
pub use session::{
    ChatMessage, DrainOutcome, OutEvent, Responder, SendOutcome, Session, SessionManager,
    StubResponder, is_transition_allowed, OP_TIMEOUT, STUB_DELAY,
};
