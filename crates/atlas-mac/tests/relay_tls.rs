use atlas_mac::*;
use serde::Deserialize;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SERVER: &str = r#"
import json
import sys
import threading
from test_hefesto import TLSFixture, DEVICE, CONTROLLER
from hefesto import Journal, create_server

fixture = TLSFixture()
journal = server = thread = None

def start(port=0):
    global journal, server, thread
    journal = Journal(str(fixture.root / 'cross.sqlite'))
    server = create_server(('127.0.0.1', port), journal,
        {fixture.fingerprint('device'): DEVICE,
         fixture.fingerprint('controller'): CONTROLLER},
        certfile=fixture.file('server', '.pem'),
        keyfile=fixture.file('server', '.key'),
        cafile=fixture.file('ca', '.pem'))
    thread = threading.Thread(target=server.serve_forever)
    thread.start()

def stop():
    global journal, server, thread
    if server is not None:
        if thread is not None:
            server.shutdown()
            thread.join()
        server.server_close()
    if journal is not None:
        journal.close()
    journal = server = thread = None

try:
    start()
    print(json.dumps({'port': server.server_port, 'directory': str(fixture.root)}), flush=True)
    for line in sys.stdin:
        if line.strip() != 'restart':
            raise ValueError('invalid_test_command')
        port = server.server_port
        stop()
        start(port)
        print('restarted', flush=True)
finally:
    stop()
    fixture.close()
"#;

#[derive(Deserialize)]
struct Metadata {
    port: u16,
    directory: PathBuf,
}

struct Relay {
    child: Child,
    output: BufReader<ChildStdout>,
}

impl Relay {
    fn start() -> (Self, Metadata) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut child = Command::new("python3")
            .args(["-u", "-c", SERVER])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("PYTHONPATH", root.join("server"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start isolated Python TLS fixture");
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut relay = Self { child, output };
        let metadata = serde_json::from_str(&relay.line()).expect("TLS fixture metadata");
        (relay, metadata)
    }

    fn line(&mut self) -> String {
        let mut line = String::new();
        assert!(self.output.read_line(&mut line).unwrap() > 0);
        line.trim().to_owned()
    }

    fn restart(&mut self) {
        self.child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"restart\n")
            .unwrap();
        assert_eq!(self.line(), "restarted");
    }

    fn stop(&mut self) {
        self.child.stdin.take();
        assert!(self.child.wait().unwrap().success());
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.child.stdin.take();
        let _ = self.child.wait();
    }
}

fn id(value: &str) -> OpaqueId {
    OpaqueId::new(value).unwrap()
}

fn clock(start: Instant) -> Clock {
    Clock::new(
        u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap(),
        start.elapsed(),
    )
    .unwrap()
}

fn client(metadata: &Metadata, name: &str, ca: &str) -> HefestoClient {
    let mut identity = fs::read(metadata.directory.join(format!("{name}.pem"))).unwrap();
    identity.extend(fs::read(metadata.directory.join(format!("{name}.key"))).unwrap());
    HefestoClient::new(
        &format!("https://127.0.0.1:{}", metadata.port),
        &fs::read(metadata.directory.join(format!("{ca}.pem"))).unwrap(),
        &identity,
        Duration::from_secs(3),
    )
    .unwrap()
}

fn submission(value: &str) -> SubmitRequest {
    SubmitRequest {
        protocol_version: ProtocolVersion,
        submission_id: id(value),
        request_id: id("request-cross"),
        session_id: id("session-cross"),
        task_id: id("task-cross"),
        tool_id: ToolId::Echo,
        arguments: ToolArguments::Echo(EchoPayload::new("héllo 世界").unwrap()),
    }
}

fn test_roots() -> Roots {
    Roots::new(vec![std::env::temp_dir()])
}

fn resume(executor: &mut Executor, client: &HefestoClient, start: Instant) {
    let attempt = executor.prepare_resume(clock(start)).unwrap();
    let status = client.control(attempt.request()).unwrap();
    executor
        .acknowledge_resume(attempt, &status, clock(start))
        .unwrap();
}

#[test]
fn python_rust_mtls_echo_abort_and_restart() {
    let (mut relay, metadata) = Relay::start();
    let device = client(&metadata, "device", "ca");
    let controller = client(&metadata, "controller", "ca");
    let initial = controller.status().unwrap();
    assert_eq!(initial.generation.get(), 0);
    assert!(!initial.online);
    assert!(initial.suspended);
    assert_eq!(
        controller.submit(&submission("absent")),
        Err(Error::RemoteRejected)
    );
    assert_eq!(
        client(&metadata, "unlisted", "ca").status(),
        Err(Error::Transport)
    );
    assert_eq!(
        client(&metadata, "device", "foreign-ca").status(),
        Err(Error::Transport)
    );
    assert_eq!(
        controller.register(&RegisterRequest::new(id("wrong-role"))),
        Err(Error::RemoteRejected)
    );
    assert_eq!(
        device.submit(&submission("wrong-role")),
        Err(Error::RemoteRejected)
    );

    let start = Instant::now();
    let instance = id("rust-instance");
    let registration = RegisterRequest::new(instance.clone());
    let mut executor = Executor::new(id("mac-one"), instance.clone(), test_roots());
    let attempt = executor.prepare_registration(clock(start)).unwrap();
    let registered = device.register(&registration).unwrap();
    executor
        .acknowledge_registration(attempt, &registered, clock(start))
        .unwrap();
    assert!(executor.is_suspended());
    resume(&mut executor, &device, start);

    let request = submission("first");
    let pending = controller.submit(&request).unwrap();
    assert_eq!(pending.state, InvocationState::Pending);
    assert_eq!(controller.submit(&request).unwrap(), pending);
    let next = NextRequest::new(instance.clone(), executor.generation().unwrap());
    let invocation = device.next(&next).unwrap().invocation.unwrap();
    let result = executor.execute(&invocation, clock(start)).unwrap();
    let duplicate = device.next(&next).unwrap().invocation.unwrap();
    assert_eq!(executor.execute(&duplicate, clock(start)).unwrap(), result);
    assert_eq!(executor.completed_count(), 1);
    let mut changed = invocation.clone();
    changed.arguments = ToolArguments::Echo(EchoPayload::new("changed").unwrap());
    assert_eq!(
        executor.execute(&changed, clock(start)),
        Err(Error::InvocationConflict)
    );
    let mut changed_result = result.clone();
    changed_result.output = OutputPayload {
        text: TextOutput::new("changed").unwrap(),
    };
    assert_eq!(device.result(&changed_result), Err(Error::RemoteRejected));
    let succeeded = device.result(&result).unwrap();
    assert_eq!(succeeded.state, InvocationState::Succeeded);
    assert_eq!(device.result(&result).unwrap(), succeeded);

    let sent = clock(start);
    let heartbeat = device.heartbeat(&registration).unwrap();
    executor
        .acknowledge_heartbeat(&heartbeat, sent, clock(start))
        .unwrap();
    let cancelled_request = submission("cancelled");
    controller.submit(&cancelled_request).unwrap();
    executor.local_abort();
    assert_eq!(
        executor.execute(&invocation, clock(start)),
        Err(Error::Suspended)
    );
    let aborted = device
        .control(&ControlRequest::new(
            ControlAction::Abort,
            executor.generation().unwrap(),
        ))
        .unwrap();
    executor.observe_status(&aborted, clock(start)).unwrap();
    assert_eq!(
        controller.submit(&cancelled_request).unwrap().state,
        InvocationState::Cancelled
    );
    resume(&mut executor, &device, start);
    assert_eq!(
        executor.execute(&invocation, clock(start)),
        Err(Error::GenerationMismatch)
    );
    let next = NextRequest::new(instance.clone(), executor.generation().unwrap());
    assert!(device.next(&next).unwrap().invocation.is_none());

    let interrupted = submission("interrupted");
    controller.submit(&interrupted).unwrap();
    let old_delivery = device.next(&next).unwrap().invocation.unwrap();
    relay.restart();
    let offline = device.status().unwrap();
    assert!(!offline.online);
    assert!(offline.suspended);
    assert!(executor.observe_status(&offline, clock(start)).is_err());
    assert!(executor.is_suspended());
    assert_eq!(device.register(&registration), Err(Error::RemoteRejected));
    assert_eq!(
        controller.submit(&interrupted).unwrap().state,
        InvocationState::Unknown
    );
    assert_eq!(device.result(&result).unwrap(), succeeded);
    assert_eq!(
        controller.submit(&submission("offline-new")),
        Err(Error::RemoteRejected)
    );

    let fresh_instance = id("rust-new-instance");
    let mut fresh = Executor::new(id("mac-one"), fresh_instance.clone(), test_roots());
    let attempt = fresh.prepare_registration(clock(start)).unwrap();
    let registered = device
        .register(&RegisterRequest::new(fresh_instance.clone()))
        .unwrap();
    fresh
        .acknowledge_registration(attempt, &registered, clock(start))
        .unwrap();
    resume(&mut fresh, &device, start);
    assert_eq!(
        fresh.execute(&old_delivery, clock(start)),
        Err(Error::IdentityMismatch)
    );
    assert!(device
        .next(&NextRequest::new(
            fresh_instance,
            fresh.generation().unwrap()
        ))
        .unwrap()
        .invocation
        .is_none());
    relay.stop();
    assert!(!metadata.directory.exists());
}
