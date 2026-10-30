use atlas_mac::*;
use std::time::Duration;

const WALL: u64 = 1_700_000_000_000;

fn id(value: &str) -> OpaqueId {
    OpaqueId::new(value).unwrap()
}
fn generation(value: u64) -> Generation {
    Generation::new(value).unwrap()
}
fn at(millis: u64) -> Clock {
    wall_at(WALL + millis, millis)
}
fn wall_at(wall: u64, millis: u64) -> Clock {
    Clock::new(wall, Duration::from_millis(millis)).unwrap()
}

fn status(gen: u64, suspended: bool) -> Status {
    Status {
        protocol_version: ProtocolVersion,
        device_id: id("device"),
        instance_id: Some(id("instance")),
        generation: generation(gen),
        suspended,
        online: true,
        active_invocation_id: None,
    }
}

fn invocation() -> Invocation {
    Invocation {
        protocol_version: ProtocolVersion,
        invocation_id: id("invocation"),
        request_id: id("request"),
        session_id: id("session"),
        task_id: id("task"),
        device_id: id("device"),
        instance_id: id("instance"),
        generation: generation(2),
        tool_id: ToolId::Echo,
        arguments: ToolArguments::Echo(EchoPayload::new("hello").unwrap()),
        expires_at_unix_ms: UnixMillis::new(WALL + 15_000).unwrap(),
    }
}

fn registered() -> Executor {
    let mut executor = Executor::new(
        id("device"),
        id("instance"),
        Roots::new(vec![std::env::temp_dir()]),
    );
    let attempt = executor.prepare_registration(at(0)).unwrap();
    executor
        .acknowledge_registration(attempt, &status(1, true), at(0))
        .unwrap();
    executor
}

fn ready() -> Executor {
    let mut executor = registered();
    let attempt = executor.prepare_resume(at(1)).unwrap();
    assert_eq!(attempt.request().action, ControlAction::Resume);
    assert_eq!(attempt.request().expected_generation, generation(1));
    executor
        .acknowledge_resume(attempt, &status(2, false), at(2))
        .unwrap();
    executor
}

#[test]
fn starts_suspended_and_only_manual_acknowledgement_enables_echo() {
    let mut executor = Executor::new(
        id("device"),
        id("instance"),
        Roots::new(vec![std::env::temp_dir()]),
    );
    assert!(executor.is_suspended());
    assert_eq!(executor.execute(&invocation(), at(0)), Err(Error::Offline));
    assert!(executor.prepare_resume(at(0)).is_err());
    let attempt = executor.prepare_registration(at(0)).unwrap();
    executor
        .acknowledge_registration(attempt, &status(1, true), at(0))
        .unwrap();
    assert_eq!(
        executor.execute(&invocation(), at(1)),
        Err(Error::Suspended)
    );
    executor.observe_status(&status(2, false), at(2)).unwrap();
    assert!(executor.is_suspended());
    executor
        .acknowledge_heartbeat(&status(2, false), at(3), at(4))
        .unwrap();
    assert!(executor.is_suspended());
    let attempt = executor.prepare_resume(at(5)).unwrap();
    executor
        .acknowledge_resume(attempt, &status(3, false), at(6))
        .unwrap();
    let mut task = invocation();
    task.generation = generation(3);
    let result = executor.execute(&task, at(7)).unwrap();
    assert_eq!(result.output.text.as_str(), "hello");
}

#[test]
fn registration_must_be_suspended_and_cannot_be_reused() {
    let mut executor = Executor::new(
        id("device"),
        id("instance"),
        Roots::new(vec![std::env::temp_dir()]),
    );
    let attempt = executor.prepare_registration(at(0)).unwrap();
    assert_eq!(
        executor.acknowledge_registration(attempt, &status(1, false), at(0)),
        Err(Error::InvalidState)
    );
    assert!(executor.is_suspended());
    let mut executor = registered();
    assert_eq!(
        executor.prepare_registration(at(14_000)).err(),
        Some(Error::AlreadyRegistered)
    );
    assert_eq!(
        executor.prepare_resume(at(15_000)).err(),
        Some(Error::LeaseExpired)
    );
}

#[test]
fn every_identity_and_generation_is_checked_before_execution() {
    for field in ["device", "instance", "generation"] {
        let mut executor = ready();
        let mut task = invocation();
        let error = match field {
            "device" => {
                task.device_id = id("other");
                Error::IdentityMismatch
            }
            "instance" => {
                task.instance_id = id("other");
                Error::IdentityMismatch
            }
            _ => {
                task.generation = generation(3);
                Error::GenerationMismatch
            }
        };
        assert_eq!(executor.execute(&task, at(10)), Err(error));
        assert_eq!(executor.completed_count(), 0);
    }
}

#[test]
fn wrong_status_identity_regression_and_semantics_fail_closed() {
    for field in ["device", "instance", "generation", "semantic"] {
        let mut executor = ready();
        let mut wrong = status(2, false);
        let error = match field {
            "device" => {
                wrong.device_id = id("other");
                Error::IdentityMismatch
            }
            "instance" => {
                wrong.instance_id = Some(id("other"));
                Error::IdentityMismatch
            }
            "generation" => {
                wrong.generation = generation(1);
                Error::GenerationMismatch
            }
            _ => {
                wrong.online = false;
                Error::InvalidState
            }
        };
        assert_eq!(executor.observe_status(&wrong, at(10)), Err(error));
        assert!(executor.is_suspended());
        assert!(executor.execute(&invocation(), at(11)).is_err());
        assert_eq!(executor.generation(), Some(generation(2)));
    }
}

#[test]
fn registration_rejects_mismatched_identity() {
    let mut executor = Executor::new(
        id("device"),
        id("instance"),
        Roots::new(vec![std::env::temp_dir()]),
    );
    let mut wrong = status(1, true);
    wrong.device_id = id("other");
    let attempt = executor.prepare_registration(at(0)).unwrap();
    assert_eq!(
        executor.acknowledge_registration(attempt, &wrong, at(0)),
        Err(Error::IdentityMismatch)
    );
    assert!(executor.is_suspended());
    assert_eq!(executor.generation(), None);
}

#[test]
fn identical_delivery_returns_exact_cached_result_without_reexecution() {
    let mut executor = ready();
    let task = invocation();
    let first = executor.execute(&task, at(10)).unwrap();
    for time in 11..100 {
        assert_eq!(executor.execute(&task, at(time)).unwrap(), first);
        assert_eq!(executor.completed_count(), 1);
    }
    assert_eq!(first.output.text.as_str(), "hello");
    assert_eq!(first.instance_id, task.instance_id);
    assert_eq!(first.generation, task.generation);
    assert_eq!(first.invocation_id, task.invocation_id);
}

#[test]
fn same_id_with_any_changed_payload_is_rejected_without_losing_result() {
    for field in ["text", "request", "session", "task", "expiration"] {
        let mut executor = ready();
        let original = invocation();
        let result = executor.execute(&original, at(10)).unwrap();
        let mut changed = original.clone();
        match field {
            "text" => changed.arguments = ToolArguments::Echo(EchoPayload::new("mutated").unwrap()),
            "request" => changed.request_id = id("other"),
            "session" => changed.session_id = id("other"),
            "task" => changed.task_id = id("other"),
            _ => changed.expires_at_unix_ms = UnixMillis::new(WALL + 14_000).unwrap(),
        }
        assert_eq!(
            executor.execute(&changed, at(11)),
            Err(Error::InvocationConflict)
        );
        assert_eq!(executor.execute(&original, at(12)).unwrap(), result);
        assert_eq!(executor.completed_count(), 1);
    }
}

#[test]
fn results_are_retained_for_the_instance_lifetime_not_just_latest_task() {
    let mut executor = ready();
    let first_task = invocation();
    let first_result = executor.execute(&first_task, at(10)).unwrap();
    for number in 0..50 {
        let mut task = invocation();
        task.invocation_id = id(&format!("invocation_{number}"));
        executor.execute(&task, at(11 + number)).unwrap();
    }
    assert_eq!(
        executor.execute(&first_task, at(100)).unwrap(),
        first_result
    );
    assert_eq!(executor.completed_count(), 51);
    executor.local_abort();
    assert_eq!(executor.completed_count(), 51);
}

#[test]
fn local_abort_invalidates_pending_resume_and_ignores_unsuspended_updates() {
    let mut executor = registered();
    let old_attempt = executor.prepare_resume(at(1)).unwrap();
    executor.local_abort();
    assert_eq!(
        executor.acknowledge_resume(old_attempt, &status(2, false), at(2)),
        Err(Error::StaleAcknowledgement)
    );
    executor.observe_status(&status(2, false), at(3)).unwrap();
    executor
        .acknowledge_heartbeat(&status(2, false), at(4), at(5))
        .unwrap();
    assert!(executor.is_suspended());
    assert_eq!(
        executor.execute(&invocation(), at(6)),
        Err(Error::Suspended)
    );
}

#[test]
fn aborted_resume_generation_cannot_be_rearmed_for_an_old_ack() {
    let mut executor = registered();
    let pending = executor.prepare_resume(at(1)).unwrap();
    executor.local_abort();
    assert_eq!(
        executor.prepare_resume(at(2)).err(),
        Some(Error::StaleAcknowledgement)
    );
    assert_eq!(
        executor.acknowledge_resume(pending, &status(2, false), at(3)),
        Err(Error::StaleAcknowledgement)
    );
    executor.observe_status(&status(3, true), at(4)).unwrap();
    let fresh = executor.prepare_resume(at(5)).unwrap();
    executor
        .acknowledge_resume(fresh, &status(4, false), at(6))
        .unwrap();
    assert!(!executor.is_suspended());
}

#[test]
fn later_manual_attempt_invalidates_earlier_attempt() {
    let mut executor = registered();
    let first = executor.prepare_resume(at(1)).unwrap();
    let second = executor.prepare_resume(at(2)).unwrap();
    assert_eq!(
        executor.acknowledge_resume(first, &status(2, false), at(3)),
        Err(Error::StaleAcknowledgement)
    );
    executor
        .acknowledge_resume(second, &status(2, false), at(4))
        .unwrap();
    assert!(!executor.is_suspended());
}

#[test]
fn manual_resume_ack_requires_identity_exact_next_generation_and_online_state() {
    for field in [
        "device",
        "instance",
        "generation",
        "unchanged",
        "suspended",
        "offline",
    ] {
        let mut executor = registered();
        let attempt = executor.prepare_resume(at(1)).unwrap();
        let mut response = status(2, false);
        match field {
            "device" => response.device_id = id("other"),
            "instance" => response.instance_id = Some(id("other")),
            "generation" => response.generation = generation(3),
            "unchanged" => response.generation = generation(1),
            "suspended" => response.suspended = true,
            _ => {
                response.online = false;
                response.suspended = true;
            }
        }
        assert!(executor
            .acknowledge_resume(attempt, &response, at(2))
            .is_err());
        assert!(executor.is_suspended());
    }
}

#[test]
fn abort_resume_cannot_recover_old_or_previously_unseen_work() {
    let mut executor = ready();
    let old_task = invocation();
    executor.execute(&old_task, at(10)).unwrap();
    executor.local_abort();
    assert_eq!(executor.execute(&old_task, at(11)), Err(Error::Suspended));
    executor.observe_status(&status(3, true), at(12)).unwrap();
    let attempt = executor.prepare_resume(at(13)).unwrap();
    executor
        .acknowledge_resume(attempt, &status(4, false), at(14))
        .unwrap();
    assert_eq!(
        executor.execute(&old_task, at(15)),
        Err(Error::GenerationMismatch)
    );
    let mut unseen = old_task.clone();
    unseen.invocation_id = id("unseen");
    assert_eq!(
        executor.execute(&unseen, at(16)),
        Err(Error::GenerationMismatch)
    );
    let mut reused_id = old_task;
    reused_id.generation = generation(4);
    assert_eq!(
        executor.execute(&reused_id, at(17)),
        Err(Error::InvocationConflict)
    );
    assert_eq!(executor.completed_count(), 1);
}

#[test]
fn changed_generation_suspends_even_if_server_claims_running() {
    let mut executor = ready();
    executor.observe_status(&status(3, false), at(10)).unwrap();
    assert!(executor.is_suspended());
    assert_eq!(executor.generation(), Some(generation(3)));
    assert_eq!(
        executor.execute(&invocation(), at(11)),
        Err(Error::Suspended)
    );
}

#[test]
fn heartbeat_renews_only_a_live_lease_and_never_implicitly_resumes() {
    let mut executor = ready();
    executor
        .acknowledge_heartbeat(&status(2, false), at(14_000), at(14_100))
        .unwrap();
    let mut task = invocation();
    task.expires_at_unix_ms = UnixMillis::new(WALL + 30_000).unwrap();
    assert!(executor.execute(&task, at(20_000)).is_ok());
    executor.local_abort();
    executor
        .acknowledge_heartbeat(&status(2, false), at(21_000), at(21_100))
        .unwrap();
    assert!(executor.is_suspended());
    assert_eq!(executor.execute(&task, at(22_000)), Err(Error::Suspended));
}

#[test]
fn delayed_heartbeat_ack_is_bounded_from_send_not_receive() {
    let mut executor = ready();
    executor
        .acknowledge_heartbeat(&status(2, false), at(10_000), at(14_999))
        .unwrap();
    let mut task = invocation();
    task.expires_at_unix_ms = UnixMillis::new(WALL + 60_000).unwrap();
    assert!(executor.execute(&task, at(24_999)).is_ok());
    assert_eq!(
        executor.execute(&task, at(25_000)),
        Err(Error::LeaseExpired)
    );
}

#[test]
fn duplicate_heartbeat_and_status_cannot_extend_lease() {
    let mut executor = ready();
    executor
        .acknowledge_heartbeat(&status(2, false), at(10_000), at(10_001))
        .unwrap();
    executor
        .acknowledge_heartbeat(&status(2, false), at(10_000), at(24_000))
        .unwrap();
    executor
        .observe_status(&status(2, false), at(24_999))
        .unwrap();
    assert_eq!(
        executor.execute(&invocation(), at(25_000)),
        Err(Error::LeaseExpired)
    );
}

#[test]
fn stale_heartbeat_does_not_renew_or_regress_generation() {
    let mut executor = ready();
    executor
        .acknowledge_heartbeat(&status(2, false), at(10_000), at(10_001))
        .unwrap();
    assert_eq!(
        executor.acknowledge_heartbeat(&status(2, false), at(9_000), at(24_000)),
        Err(Error::StaleAcknowledgement)
    );
    assert_eq!(
        executor.execute(&invocation(), at(25_000)),
        Err(Error::LeaseExpired)
    );
}

#[test]
fn expiration_permanently_retires_instance_even_with_new_online_ack() {
    let mut executor = ready();
    assert_eq!(
        executor.execute(&invocation(), at(15_000)),
        Err(Error::LeaseExpired)
    );
    assert_eq!(
        executor.acknowledge_heartbeat(&status(2, false), at(15_001), at(15_002)),
        Err(Error::LeaseExpired)
    );
    assert_eq!(
        executor.prepare_resume(at(15_003)).err(),
        Some(Error::LeaseExpired)
    );
    assert_eq!(
        executor.prepare_registration(at(15_004)).err(),
        Some(Error::AlreadyRegistered)
    );
    assert!(executor.is_suspended());
}

#[test]
fn late_registration_ack_is_not_a_fresh_lease() {
    let mut executor = Executor::new(
        id("device"),
        id("instance"),
        Roots::new(vec![std::env::temp_dir()]),
    );
    let attempt = executor.prepare_registration(at(0)).unwrap();
    assert_eq!(
        executor.acknowledge_registration(attempt, &status(1, true), at(15_000)),
        Err(Error::LeaseExpired)
    );
    assert_eq!(
        executor.prepare_registration(at(15_001)).err(),
        Some(Error::LeaseExpired)
    );
}

#[test]
fn lost_registration_ack_retry_cannot_extend_the_initial_lease() {
    let mut executor = Executor::new(
        id("device"),
        id("instance"),
        Roots::new(vec![std::env::temp_dir()]),
    );
    // First register sent at t=0; its ACK is lost, so the caller retries at t=10.
    // The relay anchored the lease at the first registration (t=0 -> 15) and never
    // renews a still-live ID, so the retry must not extend the local lease past 15.
    executor.prepare_registration(at(0)).unwrap();
    let retry = executor.prepare_registration(at(10_000)).unwrap();
    executor
        .acknowledge_registration(retry, &status(1, true), at(10_001))
        .unwrap();
    let attempt = executor.prepare_resume(at(14_000)).unwrap();
    executor
        .acknowledge_resume(attempt, &status(2, false), at(14_001))
        .unwrap();
    let mut task = invocation();
    task.expires_at_unix_ms = UnixMillis::new(WALL + 29_000).unwrap();
    assert!(executor.execute(&task, at(14_002)).is_ok());
    assert_eq!(
        executor.execute(&task, at(16_000)),
        Err(Error::LeaseExpired)
    );
}

#[test]
fn invocation_expiration_is_checked_on_first_and_duplicate_delivery() {
    let mut executor = ready();
    let mut task = invocation();
    task.expires_at_unix_ms = UnixMillis::new(WALL + 20).unwrap();
    assert!(executor.execute(&task, at(19)).is_ok());
    assert_eq!(
        executor.execute(&task, at(20)),
        Err(Error::InvocationExpired)
    );
    task.invocation_id = id("unseen");
    assert_eq!(
        executor.execute(&task, at(21)),
        Err(Error::InvocationExpired)
    );
    assert_eq!(executor.completed_count(), 1);
}

#[test]
fn duplicate_delivery_never_extends_monotonic_invocation_deadline() {
    let mut executor = ready();
    let mut task = invocation();
    task.expires_at_unix_ms = UnixMillis::new(WALL + 60_000).unwrap();
    let result = executor.execute(&task, at(1_000)).unwrap();
    executor
        .acknowledge_heartbeat(&status(2, false), at(14_000), at(14_001))
        .unwrap();
    assert_eq!(executor.execute(&task, at(15_999)).unwrap(), result);
    assert_eq!(
        executor.execute(&task, wall_at(WALL - 60_000, 16_000)),
        Err(Error::InvocationExpired)
    );
    assert_eq!(executor.completed_count(), 1);
}

#[test]
fn wall_clock_rollback_cannot_extend_lease_or_revive_expired_invocation() {
    let mut executor = ready();
    let mut task = invocation();
    task.expires_at_unix_ms = UnixMillis::new(WALL + 100).unwrap();
    assert_eq!(
        executor.execute(&task, at(100)),
        Err(Error::InvocationExpired)
    );
    assert_eq!(
        executor.execute(&task, wall_at(WALL, 101)),
        Err(Error::InvocationExpired)
    );
    let task = invocation();
    assert_eq!(
        executor.execute(&task, wall_at(WALL - 60_000, 15_000)),
        Err(Error::LeaseExpired)
    );
}

#[test]
fn wall_clock_forward_jump_is_sticky_even_at_same_monotonic_time() {
    let mut executor = ready();
    let mut task = invocation();
    task.expires_at_unix_ms = UnixMillis::new(WALL + 1_000).unwrap();
    assert_eq!(
        executor.execute(&task, wall_at(WALL + 2_000, 10)),
        Err(Error::InvocationExpired)
    );
    assert_eq!(
        executor.execute(&task, wall_at(WALL, 10)),
        Err(Error::InvocationExpired)
    );
    assert_eq!(
        executor.execute(&invocation(), wall_at(WALL + 15_000, 11)),
        Err(Error::LeaseExpired)
    );
}

#[test]
fn request_clock_high_water_survives_rollback_before_ack() {
    let mut executor = ready();
    executor
        .acknowledge_heartbeat(
            &status(2, false),
            wall_at(WALL + 2_000, 10),
            wall_at(WALL, 11),
        )
        .unwrap();
    let mut task = invocation();
    task.expires_at_unix_ms = UnixMillis::new(WALL + 1_000).unwrap();
    assert_eq!(
        executor.execute(&task, wall_at(WALL, 12)),
        Err(Error::InvocationExpired)
    );
}

#[test]
fn submillisecond_ticks_do_not_discard_monotonic_time() {
    let mut executor = ready();
    let mut task = invocation();
    task.expires_at_unix_ms = UnixMillis::new(WALL + 3).unwrap();
    for micros in [2_100, 2_200, 2_500, 2_900] {
        let now = Clock::new(WALL, Duration::from_micros(micros)).unwrap();
        assert!(executor.execute(&task, now).is_ok());
    }
    assert_eq!(
        executor.execute(&task, wall_at(WALL, 3)),
        Err(Error::InvocationExpired)
    );
}

#[test]
fn backwards_monotonic_clock_is_fail_closed_and_permanent() {
    let mut executor = ready();
    assert!(executor.execute(&invocation(), at(10)).is_ok());
    assert_eq!(
        executor.execute(&invocation(), at(9)),
        Err(Error::InvalidClock)
    );
    assert!(executor.is_suspended());
    assert_eq!(
        executor.execute(&invocation(), at(11)),
        Err(Error::LeaseExpired)
    );
}

#[test]
fn invalid_clock_and_integer_overflow_cannot_grant_a_lease() {
    let mut executor = ready();
    assert_eq!(
        executor.acknowledge_heartbeat(&status(2, false), at(11), at(10)),
        Err(Error::InvalidClock)
    );
    assert!(executor.is_suspended());
    let mut executor = Executor::new(
        id("device"),
        id("instance"),
        Roots::new(vec![std::env::temp_dir()]),
    );
    let now = wall_at(MAX_WIRE_INTEGER, 0);
    let attempt = executor.prepare_registration(now).unwrap();
    assert_eq!(
        executor.acknowledge_registration(attempt, &status(1, true), now),
        Err(Error::InvalidExpiration)
    );
    assert!(executor.is_suspended());
    let mut executor = Executor::new(
        id("device"),
        id("instance"),
        Roots::new(vec![std::env::temp_dir()]),
    );
    let now = Clock::new(WALL, Duration::MAX).unwrap();
    let attempt = executor.prepare_registration(now).unwrap();
    assert_eq!(
        executor.acknowledge_registration(attempt, &status(1, true), now),
        Err(Error::InvalidClock)
    );
    let mut executor = Executor::new(
        id("device"),
        id("instance"),
        Roots::new(vec![std::env::temp_dir()]),
    );
    let attempt = executor.prepare_registration(at(0)).unwrap();
    executor
        .acknowledge_registration(attempt, &status(MAX_WIRE_INTEGER, true), at(0))
        .unwrap();
    assert_eq!(
        executor.prepare_resume(at(1)).err(),
        Some(Error::InvalidGeneration)
    );
}

#[test]
fn connection_failure_suspends_independently_and_heartbeat_is_not_resume() {
    let mut executor = ready();
    executor.connection_failed();
    assert!(executor.is_suspended());
    assert_eq!(executor.execute(&invocation(), at(10)), Err(Error::Offline));
    assert_eq!(executor.prepare_resume(at(11)).err(), Some(Error::Offline));
    executor
        .acknowledge_heartbeat(&status(2, false), at(12), at(13))
        .unwrap();
    assert_eq!(
        executor.execute(&invocation(), at(14)),
        Err(Error::Suspended)
    );
}

#[test]
fn offline_status_retires_instance_and_relay_restart_is_rejected() {
    let mut executor = ready();
    let mut offline = status(3, true);
    offline.online = false;
    assert_eq!(
        executor.observe_status(&offline, at(10)),
        Err(Error::LeaseExpired)
    );
    assert!(executor.is_suspended());
    assert_eq!(
        executor.observe_status(&status(4, false), at(11)),
        Err(Error::LeaseExpired)
    );
    let mut executor = ready();
    offline.instance_id = None;
    assert_eq!(
        executor.observe_status(&offline, at(10)),
        Err(Error::IdentityMismatch)
    );
    assert!(executor.is_suspended());
}

#[test]
fn dispatcher_routes_read_tools_and_rejects_mismatches() {
    let dir = std::env::temp_dir().join(format!(
        "atlas-mac-exec-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("greet.txt"), "hola mundo").unwrap();

    let mut executor = Executor::new(id("device"), id("instance"), Roots::new(vec![dir.clone()]));
    let attempt = executor.prepare_registration(at(0)).unwrap();
    executor
        .acknowledge_registration(attempt, &status(1, true), at(0))
        .unwrap();
    let attempt = executor.prepare_resume(at(1)).unwrap();
    executor
        .acknowledge_resume(attempt, &status(2, false), at(2))
        .unwrap();

    let mut read = invocation();
    read.invocation_id = id("read");
    read.tool_id = ToolId::ReadFile;
    read.arguments = ToolArguments::Path(PathPayload {
        path: ToolPath::new(dir.join("greet.txt").to_str().unwrap()).unwrap(),
    });
    let result = executor.execute(&read, at(3)).unwrap();
    assert_eq!(result.output.text.as_str(), "hola mundo");

    let mut list = invocation();
    list.invocation_id = id("list");
    list.tool_id = ToolId::ListDir;
    list.arguments = ToolArguments::Path(PathPayload {
        path: ToolPath::new(dir.to_str().unwrap()).unwrap(),
    });
    let result = executor.execute(&list, at(4)).unwrap();
    assert!(result.output.text.as_str().contains("greet.txt"));

    let mut mismatch = invocation();
    mismatch.invocation_id = id("mismatch");
    mismatch.arguments = ToolArguments::Path(PathPayload {
        path: ToolPath::new(dir.to_str().unwrap()).unwrap(),
    });
    assert_eq!(
        executor.execute(&mismatch, at(5)),
        Err(Error::InvalidArguments)
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn restart_uses_new_instance_and_never_accepts_old_delivery() {
    let mut executor = Executor::new(
        id("device"),
        id("new_instance"),
        Roots::new(vec![std::env::temp_dir()]),
    );
    let mut new_status = status(3, true);
    new_status.instance_id = Some(id("new_instance"));
    let attempt = executor.prepare_registration(at(0)).unwrap();
    executor
        .acknowledge_registration(attempt, &new_status, at(0))
        .unwrap();
    let attempt = executor.prepare_resume(at(1)).unwrap();
    new_status.generation = generation(4);
    new_status.suspended = false;
    executor
        .acknowledge_resume(attempt, &new_status, at(2))
        .unwrap();
    assert_eq!(
        executor.execute(&invocation(), at(3)),
        Err(Error::IdentityMismatch)
    );
    assert_eq!(executor.completed_count(), 0);
}
