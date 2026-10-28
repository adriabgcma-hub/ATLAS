use atlas_mac::*;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

fn invocation() -> Value {
    json!({
        "protocolVersion": 1, "invocationId": "invoke_1", "requestId": "request_1",
        "sessionId": "session_1", "taskId": "task_1", "deviceId": "device_1",
        "instanceId": "instance_1", "generation": 2, "toolId": "hefesto.echo",
        "arguments": {"text": "hello"}, "expiresAtUnixMs": 1700000015000u64
    })
}

fn status() -> Value {
    json!({
        "protocolVersion": 1, "deviceId": "device_1", "instanceId": "instance_1",
        "generation": 2, "suspended": false, "online": true, "activeInvocationId": null
    })
}

fn assert_rejected<T: DeserializeOwned>(value: Value) {
    assert!(
        serde_json::from_value::<T>(value.clone()).is_err(),
        "accepted {value}"
    );
}

fn check_strict<T: DeserializeOwned>(value: Value) {
    assert!(serde_json::from_value::<T>(value.clone()).is_ok());
    let mut unknown = value.clone();
    unknown["source"] = json!("trusted");
    assert_rejected::<T>(unknown);
    for invalid in [
        json!(true),
        json!(false),
        json!(0),
        json!(2),
        json!(-1),
        json!(1.0),
        json!("1"),
        Value::Null,
    ] {
        let mut wrong = value.clone();
        wrong["protocolVersion"] = invalid;
        assert_rejected::<T>(wrong);
    }
    for key in value.as_object().unwrap().keys() {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert_rejected::<T>(missing);
    }
}

#[test]
fn every_dto_requires_exact_version_fields_and_types() {
    check_strict::<RegisterRequest>(json!({"protocolVersion":1,"instanceId":"i"}));
    check_strict::<HeartbeatRequest>(json!({"protocolVersion":1,"instanceId":"i"}));
    check_strict::<ControlRequest>(
        json!({"protocolVersion":1,"action":"abort","expectedGeneration":2}),
    );
    check_strict::<SubmitRequest>(
        json!({"protocolVersion":1,"submissionId":"sub","requestId":"r","sessionId":"s","taskId":"t","toolId":"hefesto.echo","arguments":{"text":"hi"}}),
    );
    check_strict::<NextRequest>(json!({"protocolVersion":1,"instanceId":"i","generation":2}));
    check_strict::<ResultRequest>(
        json!({"protocolVersion":1,"instanceId":"i","generation":2,"invocationId":"v","output":{"text":"hi"}}),
    );
    check_strict::<Status>(status());
    check_strict::<Invocation>(invocation());
    check_strict::<NextResponse>(json!({"protocolVersion":1,"invocation":null}));
    check_strict::<NextResponse>(json!({"protocolVersion":1,"invocation":invocation()}));
    check_strict::<InvocationResponse>(
        json!({"protocolVersion":1,"invocationId":"v","state":"pending"}),
    );
    check_strict::<ErrorResponse>(json!({"protocolVersion":1,"error":"access_denied"}));
}

#[test]
fn ids_are_nonempty_bounded_opaque_ascii() {
    for valid in ["A", "a-Z_09", &"a".repeat(80)] {
        assert!(OpaqueId::new(valid).is_ok());
    }
    for invalid in ["", "a.b", "a/b", "a b", "a\nb", "ñ", "漢", &"a".repeat(81)] {
        assert_eq!(OpaqueId::new(invalid), Err(Error::InvalidId));
        let mut value = invocation();
        value["invocationId"] = json!(invalid);
        assert_rejected::<Invocation>(value);
    }
    for value in [Value::Null, json!(false), json!(5), json!([])] {
        assert_rejected::<OpaqueId>(value);
    }
}

#[test]
fn integer_fields_are_bounded_and_not_booleans() {
    assert_eq!(Generation::new(0).unwrap().get(), 0);
    assert_rejected::<UnixMillis>(json!(0));
    let bad = [
        json!(-1),
        json!(true),
        json!(false),
        json!(2.0),
        json!("2"),
        Value::Null,
        json!(u64::MAX),
    ];
    for invalid in bad {
        assert_rejected::<Generation>(invalid.clone());
        assert_rejected::<UnixMillis>(invalid.clone());
        for field in ["generation", "expiresAtUnixMs"] {
            let mut value = invocation();
            value[field] = invalid.clone();
            assert_rejected::<Invocation>(value);
        }
    }
    assert_eq!(
        Generation::new(MAX_WIRE_INTEGER).unwrap().checked_next(),
        Err(Error::InvalidGeneration)
    );
    assert_eq!(
        UnixMillis::new(MAX_WIRE_INTEGER).unwrap().checked_add(1),
        Err(Error::InvalidExpiration)
    );
    assert_eq!(
        UnixMillis::new(1).unwrap().checked_add(u64::MAX),
        Err(Error::InvalidExpiration)
    );
    assert_eq!(Generation::new(1).unwrap().checked_next().unwrap().get(), 2);
}

#[test]
fn echo_limit_counts_utf8_bytes_and_allows_empty() {
    assert!(EchoPayload::new("").is_ok());
    assert!(EchoPayload::new("x".repeat(256)).is_ok());
    assert!(EchoPayload::new("é".repeat(128)).is_ok());
    assert_eq!(EchoPayload::new("é".repeat(129)), Err(Error::InvalidText));
    assert_eq!(EchoPayload::new("x".repeat(257)), Err(Error::InvalidText));
    let mut value = invocation();
    value["arguments"]["text"] = json!("é".repeat(129));
    assert_rejected::<Invocation>(value);
    for value in [
        json!({"text":true}),
        json!({"text":"ok","argv":[]}),
        json!({}),
    ] {
        assert_rejected::<EchoPayload>(value);
    }
}

#[test]
fn no_post_accepts_or_serializes_device_identity() {
    fn check<T: DeserializeOwned + serde::Serialize>(body: Value) {
        let typed: T = serde_json::from_value(body.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), body);
        let mut extra = body;
        extra["deviceId"] = json!("device_1");
        assert_rejected::<T>(extra);
    }
    check::<RegisterRequest>(json!({"protocolVersion":1,"instanceId":"i"}));
    check::<HeartbeatRequest>(json!({"protocolVersion":1,"instanceId":"i"}));
    check::<ControlRequest>(json!({"protocolVersion":1,"action":"resume","expectedGeneration":2}));
    check::<SubmitRequest>(
        json!({"protocolVersion":1,"submissionId":"sub","requestId":"r","sessionId":"s","taskId":"t","toolId":"hefesto.echo","arguments":{"text":"hi"}}),
    );
    check::<NextRequest>(json!({"protocolVersion":1,"instanceId":"i","generation":2}));
    check::<ResultRequest>(
        json!({"protocolVersion":1,"instanceId":"i","generation":2,"invocationId":"v","output":{"text":"hi"}}),
    );
}

#[test]
fn closed_policy_cannot_be_overridden_by_metadata() {
    for tool in [
        "shell",
        "hefesto.shell",
        "hefesto.Echo",
        "echo",
        "hefesto.echo ",
    ] {
        let mut value = invocation();
        value["toolId"] = json!(tool);
        assert_rejected::<Invocation>(value);
    }
    for field in ["risk", "agent", "source", "approved", "argv"] {
        let mut value = invocation();
        value[field] = json!("trivial");
        assert_rejected::<Invocation>(value);
    }
    for action in ["Resume", "stop", "shell"] {
        assert_rejected::<ControlRequest>(
            json!({"protocolVersion":1,"action":action,"expectedGeneration":1}),
        );
    }
}

#[test]
fn status_validates_semantics_and_required_nulls() {
    let offline = json!({"protocolVersion":1,"deviceId":"d","instanceId":null,"generation":0,"suspended":true,"online":false,"activeInvocationId":null});
    assert!(serde_json::from_value::<Status>(offline.clone()).is_ok());
    let mut invalid = status();
    invalid["generation"] = json!(0);
    assert_rejected::<Status>(invalid);
    let mut invalid = offline.clone();
    invalid["online"] = json!(true);
    assert_rejected::<Status>(invalid);
    let mut invalid = offline.clone();
    invalid["suspended"] = json!(false);
    assert_rejected::<Status>(invalid);
    let mut invalid = offline;
    invalid["activeInvocationId"] = json!("v");
    assert_rejected::<Status>(invalid);
    let mut invalid = status();
    invalid["suspended"] = json!(true);
    invalid["activeInvocationId"] = json!("v");
    assert_rejected::<Status>(invalid);
    for flag in ["suspended", "online"] {
        let mut invalid = status();
        invalid[flag] = json!(1);
        assert_rejected::<Status>(invalid);
    }
}

#[test]
fn persisted_states_are_closed_and_nested_messages_are_strict() {
    for state in [
        "pending",
        "dispatched",
        "succeeded",
        "cancelled",
        "expired",
        "unknown",
    ] {
        assert!(serde_json::from_value::<InvocationResponse>(
            json!({"protocolVersion":1,"invocationId":"v","state":state})
        )
        .is_ok());
    }
    assert_rejected::<InvocationResponse>(
        json!({"protocolVersion":1,"invocationId":"v","state":"success"}),
    );
    let mut nested = invocation();
    nested["arguments"]["permission"] = json!("yes");
    assert_rejected::<NextResponse>(json!({"protocolVersion":1,"invocation":nested}));
    assert_rejected::<ResultRequest>(
        json!({"protocolVersion":1,"instanceId":"i","generation":1,"invocationId":"v","output":{"text":"hi","secret":"no"}}),
    );
}

#[test]
fn duplicate_keys_trailing_data_and_oversized_json_are_rejected() {
    for bytes in [
        br#"{"protocolVersion":1,"protocolVersion":1,"instanceId":"i"}"#.as_slice(),
        br#"{"protocolVersion":1,"instanceId":"i","instanceId":"j"}"#,
        br#"{"protocolVersion":1,"instanceId":"i"} {}"#,
        b"\xff",
    ] {
        assert_eq!(decode::<RegisterRequest>(bytes), Err(Error::InvalidJson));
    }
    let body = br#"{"protocolVersion":1,"instanceId":"i"}"#;
    let mut exact = body.to_vec();
    exact.resize(MAX_BODY_BYTES, b' ');
    assert!(decode::<RegisterRequest>(&exact).is_ok());
    exact.push(b' ');
    assert_eq!(
        decode::<RegisterRequest>(&exact),
        Err(Error::ResponseTooLarge)
    );
}
