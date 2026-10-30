use crate::wire::*;
use crate::{Error, Result};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Clock {
    unix_ms: UnixMillis,
    monotonic: Duration,
}

impl Clock {
    pub fn new(unix_ms: u64, monotonic: Duration) -> Result<Self> {
        Ok(Self {
            unix_ms: UnixMillis::new(unix_ms)?,
            monotonic,
        })
    }

    pub fn unix_ms(self) -> UnixMillis {
        self.unix_ms
    }
    pub fn monotonic(self) -> Duration {
        self.monotonic
    }
}

#[derive(Clone, Copy)]
struct Deadline {
    wall: UnixMillis,
    monotonic: Duration,
}

impl Deadline {
    fn from_start(start: Clock, millis: u64) -> Result<Self> {
        Ok(Self {
            wall: start.unix_ms.checked_add(millis)?,
            monotonic: start
                .monotonic
                .checked_add(Duration::from_millis(millis))
                .ok_or(Error::InvalidClock)?,
        })
    }

    fn is_live(self, now: Clock) -> bool {
        now.unix_ms < self.wall && now.monotonic < self.monotonic
    }
}

pub struct ResumeAttempt {
    request: ControlRequest,
    device_id: OpaqueId,
    instance_id: OpaqueId,
    revision: u64,
}

impl ResumeAttempt {
    pub fn request(&self) -> &ControlRequest {
        &self.request
    }
}

/// Proof that a register request was sent, binding the local lease to the
/// *earliest* send. A retry whose first ACK was lost therefore cannot push the
/// lease past the relay's original registration (which never renews a live ID).
pub struct RegistrationAttempt {
    sent: Clock,
    device_id: OpaqueId,
    instance_id: OpaqueId,
}

struct CompletedInvocation {
    invocation: Invocation,
    result: ResultRequest,
    deadline: Deadline,
    revision: u64,
}

pub struct Executor {
    device_id: OpaqueId,
    instance_id: OpaqueId,
    roots: crate::tools::Roots,
    generation: Option<Generation>,
    suspended: bool,
    online: bool,
    retired: bool,
    lease: Option<Deadline>,
    registration_sent: Option<Clock>,
    last_lease_start: Option<Duration>,
    clock_anchor: Option<Clock>,
    last_now: Option<Duration>,
    revision: u64,
    pending_resume: Option<u64>,
    barred_resume_through: Option<Generation>,
    completed: HashMap<OpaqueId, CompletedInvocation>,
}

impl Executor {
    // The owner supplies a fresh instance ID for each process lifetime.
    pub fn new(device_id: OpaqueId, instance_id: OpaqueId, roots: crate::tools::Roots) -> Self {
        Self {
            device_id,
            instance_id,
            roots,
            generation: None,
            suspended: true,
            online: false,
            retired: false,
            lease: None,
            registration_sent: None,
            last_lease_start: None,
            clock_anchor: None,
            last_now: None,
            revision: 0,
            pending_resume: None,
            barred_resume_through: None,
            completed: HashMap::new(),
        }
    }

    pub fn is_suspended(&self) -> bool {
        self.suspended
    }
    pub fn generation(&self) -> Option<Generation> {
        self.generation
    }
    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }

    pub fn local_abort(&mut self) {
        let mut barred = self.generation;
        if self.pending_resume.is_some() {
            barred = barred.and_then(|generation| match generation.checked_next() {
                Ok(next) => Some(next),
                Err(_) => {
                    self.retired = true;
                    None
                }
            });
        }
        self.barred_resume_through = self.barred_resume_through.max(barred);
        self.suspend_local();
    }

    fn suspend_local(&mut self) {
        self.suspended = true;
        self.pending_resume = None;
        match self.revision.checked_add(1) {
            Some(revision) => self.revision = revision,
            None => self.retired = true,
        }
    }

    pub fn connection_failed(&mut self) {
        self.online = false;
        self.local_abort();
    }

    /// Records the moment a register request is first sent and returns a token the
    /// caller must hand back when the ACK arrives. The token binds the local lease to
    /// the *earliest* send, so a retry after a lost ACK cannot extend the lease past
    /// the relay's first registration — the relay never renews a still-live instance.
    pub fn prepare_registration(&mut self, now: Clock) -> Result<RegistrationAttempt> {
        if self.lease.is_some() {
            return Err(Error::AlreadyRegistered);
        }
        if self.retired {
            return Err(Error::LeaseExpired);
        }
        let sent = *self.registration_sent.get_or_insert(now);
        Ok(RegistrationAttempt {
            sent,
            device_id: self.device_id.clone(),
            instance_id: self.instance_id.clone(),
        })
    }

    pub fn acknowledge_registration(
        &mut self,
        attempt: RegistrationAttempt,
        status: &Status,
        now: Clock,
    ) -> Result<()> {
        if self.lease.is_some() {
            return Err(Error::AlreadyRegistered);
        }
        if self.retired {
            return Err(Error::LeaseExpired);
        }
        if attempt.device_id != self.device_id || attempt.instance_id != self.instance_id {
            return Err(Error::IdentityMismatch);
        }
        let lease = self.request_lease(attempt.sent, now)?;
        let now = self.tick(now)?;
        if !lease.is_live(now) {
            return self.expire();
        }
        self.apply_status(status)?;
        if !status.suspended {
            self.local_abort();
            return Err(Error::InvalidState);
        }
        self.lease = Some(lease);
        self.last_lease_start = Some(attempt.sent.monotonic);
        Ok(())
    }

    pub fn acknowledge_heartbeat(
        &mut self,
        status: &Status,
        sent_at: Clock,
        now: Clock,
    ) -> Result<()> {
        let lease = self.request_lease(sent_at, now)?;
        let now = self.check_lease(now)?;
        self.apply_status(status)?;
        let previous = self.last_lease_start.ok_or(Error::Offline)?;
        if sent_at.monotonic < previous {
            return Err(Error::StaleAcknowledgement);
        }
        if sent_at.monotonic == previous {
            return Ok(());
        }
        if !lease.is_live(now) {
            return self.expire();
        }
        self.lease = Some(lease);
        self.last_lease_start = Some(sent_at.monotonic);
        Ok(())
    }

    // GET/control state alone never renews the heartbeat lease or resumes execution.
    pub fn observe_status(&mut self, status: &Status, now: Clock) -> Result<()> {
        self.check_lease(now)?;
        self.apply_status(status)
    }

    pub fn prepare_resume(&mut self, now: Clock) -> Result<ResumeAttempt> {
        self.check_lease(now)?;
        if !self.online {
            return Err(Error::Offline);
        }
        let generation = self.generation.ok_or(Error::Offline)?;
        let next = generation.checked_next()?;
        if self
            .barred_resume_through
            .is_some_and(|barred| next <= barred)
        {
            return Err(Error::StaleAcknowledgement);
        }
        self.suspend_local();
        if self.retired {
            return Err(Error::LeaseExpired);
        }
        self.pending_resume = Some(self.revision);
        Ok(ResumeAttempt {
            request: ControlRequest::new(ControlAction::Resume, generation),
            device_id: self.device_id.clone(),
            instance_id: self.instance_id.clone(),
            revision: self.revision,
        })
    }

    pub fn acknowledge_resume(
        &mut self,
        attempt: ResumeAttempt,
        status: &Status,
        now: Clock,
    ) -> Result<()> {
        self.check_lease(now)?;
        if self.pending_resume != Some(attempt.revision)
            || self.revision != attempt.revision
            || self.device_id != attempt.device_id
            || self.instance_id != attempt.instance_id
            || self.generation != Some(attempt.request.expected_generation)
        {
            return Err(Error::StaleAcknowledgement);
        }
        if status.generation != attempt.request.expected_generation.checked_next()?
            || status.suspended
            || !status.online
        {
            self.local_abort();
            return Err(Error::InvalidState);
        }
        self.apply_status(status)?;
        self.pending_resume = None;
        self.suspended = false;
        Ok(())
    }

    pub fn execute(&mut self, invocation: &Invocation, now: Clock) -> Result<ResultRequest> {
        let now = self.check_lease(now)?;
        if !self.online {
            return Err(Error::Offline);
        }
        if self.suspended {
            return Err(Error::Suspended);
        }
        if invocation.device_id != self.device_id || invocation.instance_id != self.instance_id {
            return Err(Error::IdentityMismatch);
        }
        if Some(invocation.generation) != self.generation {
            return Err(Error::GenerationMismatch);
        }
        if let Some(completed) = self.completed.get(&invocation.invocation_id) {
            if &completed.invocation != invocation {
                return Err(Error::InvocationConflict);
            }
            if completed.revision != self.revision {
                return Err(Error::StaleAcknowledgement);
            }
            if !completed.deadline.is_live(now) {
                return Err(Error::InvocationExpired);
            }
            return Ok(completed.result.clone());
        }
        let remaining = invocation
            .expires_at_unix_ms
            .get()
            .checked_sub(now.unix_ms.get())
            .filter(|remaining| *remaining > 0)
            .ok_or(Error::InvocationExpired)?;
        let deadline = Deadline::from_start(now, remaining.min(LEASE_MILLIS))?;
        let output = match (&invocation.tool_id, &invocation.arguments) {
            (ToolId::Echo, ToolArguments::Echo(payload)) => {
                OutputPayload::text(payload.text.as_str())?
            }
            (ToolId::ListDir, ToolArguments::Path(payload)) => {
                OutputPayload::text(crate::tools::list_dir(&self.roots, payload.path.as_str()))?
            }
            (ToolId::ReadFile, ToolArguments::Path(payload)) => {
                OutputPayload::text(crate::tools::read_file(&self.roots, payload.path.as_str()))?
            }
            (ToolId::WriteFile, ToolArguments::Write(payload)) => {
                OutputPayload::text(crate::tools::write_file(
                    &self.roots,
                    payload.path.as_str(),
                    payload.content.as_str(),
                    payload.expected_hash.as_ref().map(|hash| hash.as_str()),
                ))?
            }
            (ToolId::Rename, ToolArguments::Rename(payload)) => OutputPayload::text(
                crate::tools::rename(&self.roots, payload.from.as_str(), payload.to.as_str()),
            )?,
            (ToolId::FileInfo, ToolArguments::Path(payload)) => {
                match crate::tools::file_info(&self.roots, payload.path.as_str()) {
                    Ok(info) => OutputPayload::Info(FileInfoOutput {
                        exists: info.exists,
                        size: info.size,
                        sha256: info.sha256.as_deref().map(ContentHash::new).transpose()?,
                    }),
                    Err(text) => OutputPayload::text(text)?,
                }
            }
            _ => return Err(Error::InvalidArguments),
        };
        let result = ResultRequest {
            protocol_version: ProtocolVersion,
            instance_id: self.instance_id.clone(),
            generation: invocation.generation,
            invocation_id: invocation.invocation_id.clone(),
            output,
        };
        self.completed.insert(
            invocation.invocation_id.clone(),
            CompletedInvocation {
                invocation: invocation.clone(),
                result: result.clone(),
                deadline,
                revision: self.revision,
            },
        );
        Ok(result)
    }

    fn apply_status(&mut self, status: &Status) -> Result<()> {
        if status.validate().is_err() {
            self.connection_failed();
            return Err(Error::InvalidState);
        }
        if status.device_id != self.device_id
            || status.instance_id.as_ref() != Some(&self.instance_id)
        {
            self.connection_failed();
            return Err(Error::IdentityMismatch);
        }
        if self
            .generation
            .is_some_and(|generation| status.generation < generation)
        {
            self.connection_failed();
            return Err(Error::GenerationMismatch);
        }
        if !status.online {
            return self.expire();
        }
        if self.generation != Some(status.generation) || status.suspended {
            self.suspend_local();
        }
        self.generation = Some(status.generation);
        self.online = true;
        Ok(())
    }

    fn request_lease(&mut self, sent_at: Clock, now: Clock) -> Result<Deadline> {
        if sent_at.monotonic > now.monotonic {
            self.connection_failed();
            return Err(Error::InvalidClock);
        }
        let start = if self
            .last_now
            .is_none_or(|previous| sent_at.monotonic >= previous)
        {
            self.tick(sent_at)
        } else {
            self.project_clock(sent_at)
        };
        let lease = start.and_then(|start| Deadline::from_start(start, LEASE_MILLIS));
        if lease.is_err() {
            self.connection_failed();
        }
        lease
    }

    fn check_lease(&mut self, now: Clock) -> Result<Clock> {
        let now = self.tick(now)?;
        if self.retired {
            return Err(Error::LeaseExpired);
        }
        let lease = self.lease.ok_or(Error::Offline)?;
        if !lease.is_live(now) {
            return self.expire();
        }
        Ok(now)
    }

    fn expire<T>(&mut self) -> Result<T> {
        self.retired = true;
        self.online = false;
        self.local_abort();
        Err(Error::LeaseExpired)
    }

    fn project_clock(&self, now: Clock) -> Result<Clock> {
        let Some(anchor) = self.clock_anchor else {
            return Ok(now);
        };
        let elapsed = now
            .monotonic
            .checked_sub(anchor.monotonic)
            .ok_or(Error::InvalidClock)?;
        let elapsed_ms = u64::try_from(elapsed.as_millis()).map_err(|_| Error::InvalidClock)?;
        let floor = anchor
            .unix_ms
            .checked_add(elapsed_ms)
            .map_err(|_| Error::InvalidClock)?;
        Ok(Clock {
            unix_ms: now.unix_ms.max(floor),
            monotonic: now.monotonic,
        })
    }

    fn tick(&mut self, now: Clock) -> Result<Clock> {
        if self
            .last_now
            .is_some_and(|previous| now.monotonic < previous)
        {
            self.retired = true;
            self.connection_failed();
            return Err(Error::InvalidClock);
        }
        let projected = match self.project_clock(now) {
            Ok(projected) => projected,
            Err(error) => {
                self.retired = true;
                self.connection_failed();
                return Err(error);
            }
        };
        let advance_anchor = match self.clock_anchor {
            None => true,
            Some(anchor) => {
                let elapsed = now.monotonic - anchor.monotonic;
                let elapsed_ms =
                    u64::try_from(elapsed.as_millis()).map_err(|_| Error::InvalidClock)?;
                now.unix_ms > anchor.unix_ms.checked_add(elapsed_ms)?
            }
        };
        if advance_anchor {
            self.clock_anchor = Some(now);
        }
        self.last_now = Some(now.monotonic);
        Ok(projected)
    }
}
