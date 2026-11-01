//! Núcleo de Atlas (Fase 3).
//!
//! Fino por diseño: contratos tipados + cliente HTTP del cerebro Python
//! (`server/atlas_relay.py`), que es el dueño de sesiones, Groq y Telegram.
//! Sin Tauri, sin sesiones locales, sin claves.

pub mod contracts;
pub mod relay;

pub use contracts::{
    codes, now_iso8601, Agent, AtlasError, ConfirmationRequest, Correlation, EventType,
    MessageAppended, OutEvent, RiskLevel, Role, SessionEvent, SessionState, Source,
    MAX_CONTENT_LEN,
};
pub use relay::{
    get_events, get_session, post_cancel, post_confirm, post_send, FetchedEvents, SessionSnapshot,
    SnapshotMessage,
};
