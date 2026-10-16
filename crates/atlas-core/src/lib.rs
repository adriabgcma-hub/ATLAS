//! Núcleo de Atlas (Fase 3).
//!
//! Fino por diseño: contratos tipados + cliente HTTP del cerebro Python
//! (`server/atlas_relay.py`), que es el dueño de sesiones, Groq y Telegram.
//! Sin Tauri, sin sesiones locales, sin claves.

pub mod contracts;
pub mod relay;

pub use contracts::{
    AtlasError, Correlation, EventType, MessageAppended, OutEvent, Role, SessionEvent,
    SessionState, Source, codes, now_iso8601, MAX_CONTENT_LEN,
};
pub use relay::{
    FetchedEvents, SessionSnapshot, SnapshotMessage, get_events, get_session, post_cancel,
    post_send,
};
