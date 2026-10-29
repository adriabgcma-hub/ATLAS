//! Device runtime: keeps the Mac registered, renews the lease and executes echoes.
//!
//! The executor owns the protocol state machine; this module owns the pacing
//! (heartbeat, polling, retries) and exposes a snapshot for the UI. It never
//! starts a task the executor has not authorized.

use crate::client::HefestoClient;
use crate::executor::{Clock, EchoExecutor};
use crate::wire::*;
use crate::{Error, Result};
use serde::Serialize;
use std::time::Duration;

pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(5);
pub const POLL_EVERY: Duration = Duration::from_secs(1);
pub const RECONNECT_DELAY: Duration = Duration::from_secs(15);
pub const CONFLICT_RETRY: Duration = Duration::from_secs(5);
pub const ABORT_RETRY: Duration = Duration::from_secs(5);
pub const MAX_ABORT_ATTEMPTS: u8 = 3;

/// Phases the UI can render. `Unconfigured` never comes from the runtime: it
/// describes an app without credentials, which therefore never connects.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Unconfigured,
    Connecting,
    Suspended,
    Ready,
    Offline,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub phase: Phase,
    pub device_id: Option<String>,
    pub generation: Option<u64>,
    pub last_echo: Option<String>,
    /// Stable code only: never paths, certificates or payloads.
    pub detail: Option<&'static str>,
}

impl Snapshot {
    pub fn unconfigured() -> Self {
        Self {
            phase: Phase::Unconfigured,
            device_id: None,
            generation: None,
            last_echo: None,
            detail: None,
        }
    }

    pub fn connecting(device_id: &OpaqueId) -> Self {
        Self {
            phase: Phase::Connecting,
            device_id: Some(device_id.as_str().to_string()),
            generation: None,
            last_echo: None,
            detail: None,
        }
    }

    pub fn failed(detail: &'static str) -> Self {
        Self {
            phase: Phase::Error,
            device_id: None,
            generation: None,
            last_echo: None,
            detail: Some(detail),
        }
    }
}

pub trait Transport {
    fn register(&self, request: &RegisterRequest) -> Result<Status>;
    fn heartbeat(&self, request: &HeartbeatRequest) -> Result<Status>;
    fn control(&self, request: &ControlRequest) -> Result<Status>;
    fn next(&self, request: &NextRequest) -> Result<NextResponse>;
    fn result(&self, request: &ResultRequest) -> Result<ResultResponse>;
    fn status(&self) -> Result<Status>;
}

impl Transport for HefestoClient {
    fn register(&self, request: &RegisterRequest) -> Result<Status> {
        HefestoClient::register(self, request)
    }
    fn heartbeat(&self, request: &HeartbeatRequest) -> Result<Status> {
        HefestoClient::heartbeat(self, request)
    }
    fn control(&self, request: &ControlRequest) -> Result<Status> {
        HefestoClient::control(self, request)
    }
    fn next(&self, request: &NextRequest) -> Result<NextResponse> {
        HefestoClient::next(self, request)
    }
    fn result(&self, request: &ResultRequest) -> Result<ResultResponse> {
        HefestoClient::result(self, request)
    }
    fn status(&self) -> Result<Status> {
        HefestoClient::status(self)
    }
}

enum Failure {
    /// The relay answered and rejected us: resynchronize before deciding.
    Rejected,
    /// Transport or local state failure: this instance is no longer trustworthy.
    Lost(Error),
}

fn classify(error: Error) -> Failure {
    match error {
        Error::RemoteRejected | Error::InvalidResponse => Failure::Rejected,
        other => Failure::Lost(other),
    }
}

type Step<T> = std::result::Result<T, Failure>;

struct Session {
    instance: OpaqueId,
    executor: EchoExecutor,
}

impl Session {
    fn register<T: Transport>(&mut self, transport: &T, now: Clock) -> Step<()> {
        let attempt = self.executor.prepare_registration(now).map_err(classify)?;
        let status = transport
            .register(&RegisterRequest::new(self.instance.clone()))
            .map_err(classify)?;
        self.executor
            .acknowledge_registration(attempt, &status, now)
            .map_err(classify)
    }

    fn heartbeat<T: Transport>(&mut self, transport: &T, now: Clock) -> Step<()> {
        let status = transport
            .heartbeat(&HeartbeatRequest::new(self.instance.clone()))
            .map_err(classify)?;
        // Sending and acknowledging share the same instant: the local lease is
        // never longer than the relay's, even if the ACK takes a while.
        self.executor
            .acknowledge_heartbeat(&status, now, now)
            .map_err(classify)
    }

    fn resume<T: Transport>(&mut self, transport: &T, now: Clock) -> Step<()> {
        let attempt = self.executor.prepare_resume(now).map_err(classify)?;
        let status = transport.control(attempt.request()).map_err(classify)?;
        self.executor
            .acknowledge_resume(attempt, &status, now)
            .map_err(classify)
    }

    fn abort<T: Transport>(&mut self, transport: &T, now: Clock) -> Step<()> {
        let generation = self
            .executor
            .generation()
            .ok_or(Failure::Lost(Error::Offline))?;
        let request = ControlRequest::new(ControlAction::Abort, generation);
        let status = transport.control(&request).map_err(classify)?;
        self.executor.observe_status(&status, now).map_err(classify)
    }

    fn observe<T: Transport>(&mut self, transport: &T, now: Clock) -> Step<Status> {
        let status = transport.status().map_err(classify)?;
        self.executor
            .observe_status(&status, now)
            .map_err(classify)?;
        Ok(status)
    }

    fn poll<T: Transport>(&mut self, transport: &T, now: Clock) -> Step<Option<String>> {
        if self.executor.is_suspended() {
            return Ok(None);
        }
        let Some(generation) = self.executor.generation() else {
            return Ok(None);
        };
        let request = NextRequest::new(self.instance.clone(), generation);
        let response = transport.next(&request).map_err(classify)?;
        let Some(invocation) = response.invocation else {
            return Ok(None);
        };
        let result = match self.executor.execute(&invocation, now) {
            Ok(result) => result,
            // Delivered too late: the relay will expire it; never retry an echo.
            Err(Error::InvocationExpired) => return Ok(None),
            Err(error) => return Err(classify(error)),
        };
        transport.result(&result).map_err(classify)?;
        Ok(Some(invocation.arguments.text.as_str().to_string()))
    }
}

pub struct Runtime<T: Transport> {
    transport: T,
    device_id: OpaqueId,
    session: Option<Session>,
    phase: Phase,
    detail: Option<&'static str>,
    last_echo: Option<String>,
    registered: bool,
    last_heartbeat: Option<Duration>,
    last_poll: Option<Duration>,
    retry_at: Option<Duration>,
    pending_abort: u8,
    last_abort: Option<Duration>,
    sequence: u64,
}

impl<T: Transport> Runtime<T> {
    pub fn new(transport: T, device_id: OpaqueId) -> Self {
        Self {
            transport,
            device_id,
            session: None,
            phase: Phase::Connecting,
            detail: None,
            last_echo: None,
            registered: false,
            last_heartbeat: None,
            last_poll: None,
            retry_at: None,
            pending_abort: 0,
            last_abort: None,
            sequence: 0,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            phase: self.phase,
            device_id: Some(self.device_id.as_str().to_string()),
            generation: self
                .session
                .as_ref()
                .and_then(|session| session.executor.generation())
                .map(Generation::get),
            last_echo: self.last_echo.clone(),
            detail: self.detail,
        }
    }

    /// One paced step. The caller owns the cadence (a tick every few hundred
    /// milliseconds); this method performs at most one network round trip.
    pub fn tick(&mut self, now: Clock) {
        if let Some(retry) = self.retry_at {
            if now.monotonic() < retry {
                return;
            }
            self.retry_at = None;
        }
        if self.session.is_none() {
            if let Err(error) = self.spawn(now) {
                self.fail_start(now, error);
                return;
            }
        }
        if !self.registered {
            self.register(now);
            return;
        }
        if due(self.last_heartbeat, now.monotonic(), HEARTBEAT_EVERY) {
            self.heartbeat(now);
            return;
        }
        if self.pending_abort > 0 && due(self.last_abort, now.monotonic(), ABORT_RETRY) {
            self.retry_abort(now);
            return;
        }
        if self.phase == Phase::Ready && due(self.last_poll, now.monotonic(), POLL_EVERY) {
            self.poll(now);
        }
    }

    /// Manual activation. Only a suspended, registered device can resume.
    pub fn resume(&mut self, now: Clock) {
        if !self.registered || self.pending_abort > 0 {
            return;
        }
        let suspended = self
            .session
            .as_ref()
            .is_some_and(|session| session.executor.is_suspended());
        if !suspended {
            return;
        }
        let outcome = match self.session.as_mut() {
            Some(session) => session.resume(&self.transport, now),
            None => return,
        };
        match outcome {
            Ok(()) => {
                self.phase = Phase::Ready;
                self.detail = None;
            }
            Err(Failure::Rejected) => self.resync(now),
            Err(Failure::Lost(error)) => self.reset(now, RECONNECT_DELAY, error),
        }
    }

    /// Manual stop: effective locally even if the relay is unreachable. The
    /// remote control is retried on the following ticks and the device stays
    /// suspended until the user activates it again.
    pub fn abort(&mut self) {
        let registered = self.registered;
        if let Some(session) = self.session.as_mut() {
            session.executor.local_abort();
        }
        self.pending_abort = if registered { MAX_ABORT_ATTEMPTS } else { 0 };
        self.last_abort = None;
        self.phase = if registered {
            Phase::Suspended
        } else {
            Phase::Offline
        };
        self.detail = if registered {
            Some("abort_pending")
        } else {
            None
        };
    }

    fn spawn(&mut self, now: Clock) -> Result<()> {
        self.sequence = self.sequence.checked_add(1).ok_or(Error::InvalidId)?;
        let instance = OpaqueId::new(format!(
            "app-{}-{}-{}",
            std::process::id(),
            now.unix_ms().get(),
            self.sequence
        ))?;
        let executor = EchoExecutor::new(self.device_id.clone(), instance.clone());
        self.session = Some(Session { instance, executor });
        self.registered = false;
        self.last_heartbeat = None;
        self.last_poll = None;
        self.phase = Phase::Connecting;
        Ok(())
    }

    fn register(&mut self, now: Clock) {
        let outcome = match self.session.as_mut() {
            Some(session) => session.register(&self.transport, now),
            None => return,
        };
        match outcome {
            Ok(()) => {
                self.registered = true;
                self.phase = Phase::Suspended;
                self.detail = None;
                self.last_heartbeat = Some(now.monotonic());
            }
            Err(Failure::Rejected) => self.defer(now, CONFLICT_RETRY, "register_rejected"),
            Err(Failure::Lost(error)) => self.reset(now, RECONNECT_DELAY, error),
        }
    }

    fn heartbeat(&mut self, now: Clock) {
        let outcome = match self.session.as_mut() {
            Some(session) => session.heartbeat(&self.transport, now),
            None => return,
        };
        match outcome {
            Ok(()) => {
                self.last_heartbeat = Some(now.monotonic());
                self.sync_phase();
            }
            Err(Failure::Rejected) => self.resync(now),
            Err(Failure::Lost(error)) => self.reset(now, RECONNECT_DELAY, error),
        }
    }

    fn poll(&mut self, now: Clock) {
        let outcome = match self.session.as_mut() {
            Some(session) => session.poll(&self.transport, now),
            None => return,
        };
        self.last_poll = Some(now.monotonic());
        match outcome {
            Ok(Some(text)) => {
                self.last_echo = Some(text);
                self.sync_phase();
            }
            Ok(None) => {}
            Err(Failure::Rejected) => self.resync(now),
            Err(Failure::Lost(error)) => self.reset(now, RECONNECT_DELAY, error),
        }
    }

    fn retry_abort(&mut self, now: Clock) {
        if self.pending_abort == 0 || !self.registered {
            return;
        }
        let outcome = match self.session.as_mut() {
            Some(session) => session.abort(&self.transport, now),
            None => return,
        };
        self.last_abort = Some(now.monotonic());
        match outcome {
            Ok(()) => {
                self.pending_abort = 0;
                self.phase = Phase::Suspended;
                self.detail = None;
            }
            Err(Failure::Rejected) => {
                self.pending_abort = self.pending_abort.saturating_sub(1);
                self.resync(now);
                if self.pending_abort > 0 {
                    self.detail = Some("abort_pending");
                }
            }
            Err(Failure::Lost(error)) => {
                self.pending_abort = self.pending_abort.saturating_sub(1);
                if self.pending_abort == 0 {
                    self.reset(now, RECONNECT_DELAY, error);
                } else {
                    self.phase = Phase::Suspended;
                    self.detail = Some("abort_pending");
                }
            }
        }
    }

    /// The relay rejected an operation: ask for the authoritative state before
    /// assuming the instance is gone. Another controller may have suspended us.
    fn resync(&mut self, now: Clock) {
        let outcome = match self.session.as_mut() {
            Some(session) => session.observe(&self.transport, now),
            None => return,
        };
        match outcome {
            Ok(status) => {
                self.registered = true;
                if status.suspended {
                    self.pending_abort = 0;
                }
                self.sync_phase();
            }
            Err(Failure::Rejected) => self.reset(now, RECONNECT_DELAY, Error::RemoteRejected),
            Err(Failure::Lost(error)) => self.reset(now, RECONNECT_DELAY, error),
        }
    }

    fn sync_phase(&mut self) {
        let suspended = self
            .session
            .as_ref()
            .is_none_or(|session| session.executor.is_suspended());
        if suspended || self.pending_abort > 0 {
            self.phase = Phase::Suspended;
            if self.pending_abort > 0 {
                self.detail = Some("abort_pending");
                return;
            }
        } else {
            self.phase = Phase::Ready;
        }
        self.detail = None;
    }

    fn defer(&mut self, now: Clock, delay: Duration, code: &'static str) {
        self.phase = Phase::Connecting;
        self.detail = Some(code);
        self.retry_at = now.monotonic().checked_add(delay);
    }

    fn reset(&mut self, now: Clock, delay: Duration, error: Error) {
        if let Some(session) = self.session.as_mut() {
            session.executor.connection_failed();
        }
        self.session = None;
        self.registered = false;
        self.pending_abort = 0;
        self.phase = Phase::Offline;
        self.detail = Some(error.code());
        self.retry_at = now.monotonic().checked_add(delay);
    }

    fn fail_start(&mut self, now: Clock, error: Error) {
        self.session = None;
        self.registered = false;
        self.phase = Phase::Error;
        self.detail = Some(error.code());
        self.retry_at = now.monotonic().checked_add(RECONNECT_DELAY);
    }
}

fn due(last: Option<Duration>, now: Duration, every: Duration) -> bool {
    match last {
        None => true,
        Some(last) => now.saturating_sub(last) >= every,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    const DEVICE: &str = "mac-one";

    struct FakeState {
        instance: Option<OpaqueId>,
        generation: u64,
        suspended: bool,
        queue: VecDeque<String>,
        results: Vec<ResultRequest>,
        registers: usize,
        heartbeats: usize,
        down: bool,
        expire_next: bool,
    }

    struct Fake {
        state: RefCell<FakeState>,
        device_id: OpaqueId,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                state: RefCell::new(FakeState {
                    instance: None,
                    generation: 0,
                    suspended: true,
                    queue: VecDeque::new(),
                    results: Vec::new(),
                    registers: 0,
                    heartbeats: 0,
                    down: false,
                    expire_next: false,
                }),
                device_id: OpaqueId::new(DEVICE).unwrap(),
            }
        }

        fn status_of(state: &FakeState, device_id: &OpaqueId) -> Status {
            Status {
                protocol_version: ProtocolVersion,
                device_id: device_id.clone(),
                instance_id: state.instance.clone(),
                generation: Generation::new(state.generation).unwrap(),
                suspended: state.suspended,
                online: state.instance.is_some(),
                active_invocation_id: None,
            }
        }

        fn check(state: &FakeState) -> Result<()> {
            if state.down {
                Err(Error::Transport)
            } else {
                Ok(())
            }
        }

        fn require_instance(state: &FakeState) -> Result<()> {
            if state.instance.is_none() {
                Err(Error::RemoteRejected)
            } else {
                Ok(())
            }
        }
    }

    impl Transport for Fake {
        fn register(&self, request: &RegisterRequest) -> Result<Status> {
            let mut state = self.state.borrow_mut();
            state.registers += 1;
            Self::check(&state)?;
            match &state.instance {
                None => {
                    state.instance = Some(request.instance_id.clone());
                    state.generation += 1;
                    state.suspended = true;
                }
                Some(current) if current == &request.instance_id => {}
                Some(_) => return Err(Error::RemoteRejected),
            }
            Ok(Self::status_of(&state, &self.device_id))
        }

        fn heartbeat(&self, request: &HeartbeatRequest) -> Result<Status> {
            let mut state = self.state.borrow_mut();
            state.heartbeats += 1;
            Self::check(&state)?;
            Self::require_instance(&state)?;
            if state.instance.as_ref() != Some(&request.instance_id) {
                return Err(Error::RemoteRejected);
            }
            Ok(Self::status_of(&state, &self.device_id))
        }

        fn control(&self, request: &ControlRequest) -> Result<Status> {
            let mut state = self.state.borrow_mut();
            Self::check(&state)?;
            Self::require_instance(&state)?;
            if request.expected_generation.get() != state.generation {
                return Err(Error::RemoteRejected);
            }
            match request.action {
                ControlAction::Resume => {
                    if state.suspended {
                        state.suspended = false;
                        state.generation += 1;
                    }
                }
                ControlAction::Abort => {
                    state.suspended = true;
                    state.generation += 1;
                }
            }
            Ok(Self::status_of(&state, &self.device_id))
        }

        fn next(&self, request: &NextRequest) -> Result<NextResponse> {
            let mut state = self.state.borrow_mut();
            Self::check(&state)?;
            Self::require_instance(&state)?;
            if state.suspended
                || state.instance.as_ref() != Some(&request.instance_id)
                || state.generation != request.generation.get()
            {
                return Err(Error::RemoteRejected);
            }
            let Some(text) = state.queue.pop_front() else {
                return Ok(NextResponse {
                    protocol_version: ProtocolVersion,
                    invocation: None,
                });
            };
            let instance = state.instance.clone().unwrap();
            let invocation_id = OpaqueId::new(format!("inv-{}", state.results.len() + 1)).unwrap();
            let expires = if state.expire_next {
                UnixMillis::new(1).unwrap()
            } else {
                UnixMillis::new(9_000_000_000).unwrap()
            };
            Ok(NextResponse {
                protocol_version: ProtocolVersion,
                invocation: Some(Invocation {
                    protocol_version: ProtocolVersion,
                    invocation_id,
                    request_id: OpaqueId::new("req-1").unwrap(),
                    session_id: OpaqueId::new("ses-1").unwrap(),
                    task_id: OpaqueId::new("task-1").unwrap(),
                    device_id: self.device_id.clone(),
                    instance_id: instance,
                    generation: request.generation,
                    tool_id: ToolId::Echo,
                    arguments: EchoPayload::new(text).unwrap(),
                    expires_at_unix_ms: expires,
                }),
            })
        }

        fn result(&self, request: &ResultRequest) -> Result<ResultResponse> {
            let mut state = self.state.borrow_mut();
            Self::check(&state)?;
            state.results.push(request.clone());
            Ok(ResultResponse {
                protocol_version: ProtocolVersion,
                invocation_id: request.invocation_id.clone(),
                state: InvocationState::Succeeded,
            })
        }

        fn status(&self) -> Result<Status> {
            let state = self.state.borrow();
            Self::check(&state)?;
            Ok(Self::status_of(&state, &self.device_id))
        }
    }

    fn clock(ms: u64) -> Clock {
        Clock::new(1_000_000 + ms, Duration::from_millis(ms)).unwrap()
    }

    fn runtime() -> Runtime<Fake> {
        Runtime::new(Fake::new(), OpaqueId::new(DEVICE).unwrap())
    }

    fn instance(runtime: &Runtime<Fake>) -> OpaqueId {
        runtime.session.as_ref().unwrap().instance.clone()
    }

    #[test]
    fn registers_suspended_and_heartbeats_on_schedule() {
        let mut runtime = runtime();
        runtime.tick(clock(0));
        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.phase, Phase::Suspended);
        assert_eq!(snapshot.generation, Some(1));
        assert_eq!(runtime.transport.state.borrow().registers, 1);

        runtime.tick(clock(1_000));
        assert_eq!(runtime.transport.state.borrow().heartbeats, 0);
        runtime.tick(clock(5_000));
        assert_eq!(runtime.transport.state.borrow().heartbeats, 1);
        assert_eq!(runtime.snapshot().phase, Phase::Suspended);
    }

    #[test]
    fn resume_enables_execution_and_records_the_echo() {
        let mut runtime = runtime();
        runtime.tick(clock(0));
        runtime
            .transport
            .state
            .borrow_mut()
            .queue
            .push_back("hola".into());
        runtime.resume(clock(1_000));
        assert_eq!(runtime.snapshot().phase, Phase::Ready);
        assert_eq!(runtime.snapshot().generation, Some(2));

        runtime.tick(clock(2_000));
        let state = runtime.transport.state.borrow();
        assert_eq!(state.results.len(), 1);
        assert_eq!(state.results[0].output.text.as_str(), "hola");
        assert_eq!(runtime.snapshot().last_echo.as_deref(), Some("hola"));
    }

    #[test]
    fn expired_delivery_is_ignored_and_never_executed() {
        let mut runtime = runtime();
        runtime.tick(clock(0));
        runtime
            .transport
            .state
            .borrow_mut()
            .queue
            .push_back("tarde".into());
        runtime.transport.state.borrow_mut().expire_next = true;
        runtime.resume(clock(1_000));
        runtime.tick(clock(2_000));
        let state = runtime.transport.state.borrow();
        assert!(state.results.is_empty());
        assert_eq!(runtime.snapshot().last_echo, None);
    }

    #[test]
    fn transport_failure_goes_offline_and_reconnects_with_a_new_instance() {
        let mut runtime = runtime();
        runtime.tick(clock(0));
        let first = instance(&runtime);

        runtime.transport.state.borrow_mut().down = true;
        runtime.tick(clock(5_000));
        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.phase, Phase::Offline);
        assert_eq!(snapshot.detail, Some("transport_failed"));
        assert_eq!(snapshot.generation, None);

        runtime.tick(clock(10_000));
        assert_eq!(runtime.snapshot().phase, Phase::Offline);

        runtime.transport.state.borrow_mut().down = false;
        // The relay sweeps the instance once its 15 s lease lapses; the retry
        // (also 15 s) therefore finds no live registration under that ID.
        let mut state = runtime.transport.state.borrow_mut();
        state.instance = None;
        state.generation += 1;
        drop(state);
        runtime.tick(clock(21_000));
        let second = instance(&runtime);
        assert_ne!(first, second);
        assert_eq!(runtime.snapshot().phase, Phase::Suspended);
        assert_eq!(runtime.transport.state.borrow().registers, 2);
    }

    #[test]
    fn abort_suspends_locally_without_network_and_confirms_later() {
        let mut runtime = runtime();
        runtime.tick(clock(0));
        runtime.resume(clock(1_000));

        runtime.transport.state.borrow_mut().down = true;
        runtime.abort();
        assert_eq!(runtime.snapshot().phase, Phase::Suspended);
        assert_eq!(runtime.snapshot().detail, Some("abort_pending"));

        runtime.transport.state.borrow_mut().down = false;
        runtime.tick(clock(6_000));
        runtime.tick(clock(7_000));
        let state = runtime.transport.state.borrow();
        assert!(state.suspended);
        assert_eq!(state.generation, 3);
        assert_eq!(runtime.snapshot().phase, Phase::Suspended);
        assert_eq!(runtime.snapshot().detail, None);
    }

    #[test]
    fn remote_suspension_is_observed_instead_of_resetting() {
        let mut runtime = runtime();
        runtime.tick(clock(0));
        runtime.resume(clock(1_000));
        let current = instance(&runtime);

        {
            let mut state = runtime.transport.state.borrow_mut();
            state.suspended = true;
            state.generation += 1;
        }
        runtime.tick(clock(2_000));
        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.phase, Phase::Suspended);
        assert_eq!(instance(&runtime), current);
        assert_eq!(snapshot.generation, Some(3));
    }

    #[test]
    fn abort_without_network_gives_up_after_bounded_attempts() {
        let mut runtime = runtime();
        runtime.tick(clock(0));
        runtime.resume(clock(1_000));
        runtime.transport.state.borrow_mut().down = true;
        runtime.abort();
        runtime.tick(clock(7_000));
        runtime.tick(clock(12_000));
        assert_eq!(runtime.snapshot().phase, Phase::Offline);
        assert_eq!(runtime.snapshot().detail, Some("transport_failed"));
    }

    #[test]
    fn snapshot_contract_is_stable() {
        let mut runtime = runtime();
        runtime.tick(clock(0));
        let value = serde_json::to_value(runtime.snapshot()).unwrap();
        assert_eq!(value["phase"], "suspended");
        assert_eq!(value["deviceId"], DEVICE);
        assert_eq!(value["generation"], 1);
        assert_eq!(value["lastEcho"], serde_json::Value::Null);
        assert_eq!(value["detail"], serde_json::Value::Null);
        assert_eq!(
            serde_json::to_value(Snapshot::failed("invalid_credentials")).unwrap()["phase"],
            "error"
        );
        assert_eq!(
            serde_json::to_value(Snapshot::unconfigured()).unwrap()["phase"],
            "unconfigured"
        );
    }
}
