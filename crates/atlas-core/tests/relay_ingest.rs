//! Flujo relay → núcleo sin Tauri: fetch HTTP real contra un relay falso
//! + ingesta en `SessionManager` (mismo camino que `relay_drain`).

use atlas_core::{SessionManager, Source, fetch_messages};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

#[test]
fn relay_a_nucleo_flujo_completo() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let body = r#"{"messages":[
                {"seq": 1, "content": "hola desde telegram", "source": "telegram"},
                {"seq": 2, "content": "   ", "source": "telegram"}
            ]}"#;
            let response = format!(
                "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let fetched =
        fetch_messages(&format!("http://{addr}"), 0, Duration::from_secs(5)).expect("fetch ok");
    let mut manager = SessionManager::new();
    let outcome = manager.ingest(fetched.messages);

    assert_eq!(outcome.delivered, 1);
    assert_eq!(outcome.skipped, 1);
    let session = manager.active().expect("sesión creada");
    assert_eq!(session.messages.len(), 1);
    assert_eq!(session.messages[0].source, Source::Telegram);
    assert_eq!(session.messages[0].content, "hola desde telegram");
}
