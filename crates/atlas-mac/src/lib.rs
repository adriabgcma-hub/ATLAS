#![forbid(unsafe_code)]

pub mod client;
pub mod executor;
pub mod runtime;
pub mod tools;
pub mod wire;

pub use client::HefestoClient;
pub use executor::{Clock, Executor, RegistrationAttempt, ResumeAttempt};
pub use runtime::{Phase, Runtime, Snapshot, Transport};
pub use tools::Roots;
pub use wire::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidId,
    InvalidVersion,
    InvalidGeneration,
    InvalidExpiration,
    InvalidText,
    InvalidPath,
    InvalidHash,
    InvalidArguments,
    InvalidState,
    InvalidErrorCode,
    InvalidJson,
    InvalidUrl,
    InvalidCredentials,
    InvalidTimeout,
    Transport,
    ResponseTooLarge,
    RequestTooLarge,
    RemoteRejected,
    InvalidResponse,
    IdentityMismatch,
    GenerationMismatch,
    StaleAcknowledgement,
    Suspended,
    Offline,
    LeaseExpired,
    InvocationExpired,
    InvocationConflict,
    InvalidClock,
    AlreadyRegistered,
}

impl Error {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidId => "invalid_id",
            Self::InvalidVersion => "invalid_version",
            Self::InvalidGeneration => "invalid_generation",
            Self::InvalidExpiration => "invalid_expiration",
            Self::InvalidText => "invalid_text",
            Self::InvalidPath => "invalid_path",
            Self::InvalidHash => "invalid_hash",
            Self::InvalidArguments => "invalid_arguments",
            Self::InvalidState => "invalid_state",
            Self::InvalidErrorCode => "invalid_error_code",
            Self::InvalidJson => "invalid_json",
            Self::InvalidUrl => "invalid_url",
            Self::InvalidCredentials => "invalid_credentials",
            Self::InvalidTimeout => "invalid_timeout",
            Self::Transport => "transport_failed",
            Self::ResponseTooLarge => "response_too_large",
            Self::RequestTooLarge => "request_too_large",
            Self::RemoteRejected => "remote_rejected",
            Self::InvalidResponse => "invalid_response",
            Self::IdentityMismatch => "identity_mismatch",
            Self::GenerationMismatch => "generation_mismatch",
            Self::StaleAcknowledgement => "stale_acknowledgement",
            Self::Suspended => "suspended",
            Self::Offline => "offline",
            Self::LeaseExpired => "lease_expired",
            Self::InvocationExpired => "invocation_expired",
            Self::InvocationConflict => "invocation_conflict",
            Self::InvalidClock => "invalid_clock",
            Self::AlreadyRegistered => "already_registered",
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
