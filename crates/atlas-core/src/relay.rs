//! Cliente HTTP mínimo del cerebro Python (Fase 3). Solo std.
//!
//! El cerebro (sesiones, Groq, Telegram) vive en `server/atlas_relay.py`.
//! Este módulo solo transporta eventos y peticiones por la red local.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::Deserialize;

use super::contracts::{
    Agent, AtlasError, ConfirmationRequest, Correlation, EventType, MessageAppended, OutEvent,
    RiskLevel, Role, SessionEvent, SessionState, Source, codes,
};

fn unreachable(message: &str) -> AtlasError {
    AtlasError::new(codes::RELAY_UNREACHABLE, message, true)
}

fn invalid(message: &str) -> AtlasError {
    AtlasError::new(codes::RELAY_INVALID, message, false)
}

fn split_host_port(base_url: &str) -> Result<(String, u16), AtlasError> {
    let rest = base_url
        .strip_prefix("http://")
        .ok_or_else(|| invalid("El relay solo admite http local."))?;
    // La base es solo host[:puerto], sin rutas.
    let host_part = match rest.split_once('/') {
        Some(_) => return Err(invalid("La URL del relay no lleva ruta.")),
        None => rest,
    };
    match host_part.split_once(':') {
        Some((host, port)) => {
            let port: u16 = port
                .parse()
                .map_err(|_| invalid("Puerto del relay inválido."))?;
            Ok((host.to_string(), port))
        }
        None => Ok((host_part.to_string(), 80)),
    }
}

/// Ordena IPv4 primero: el relay escucha en la IPv4 de la LAN y el nombre
/// `.local` también resuelve a IPv6 link-local, que sin scope no conecta.
fn prefer_ipv4(mut addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    addrs.sort_by_key(SocketAddr::is_ipv6);
    addrs
}

fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>, AtlasError> {
    let addrs: Vec<SocketAddr> = format!("{host}:{port}")
        .to_socket_addrs()
        .map_err(|_| unreachable("No se resuelve el relay."))?
        .collect();
    if addrs.is_empty() {
        return Err(unreachable("Relay sin dirección."));
    }
    Ok(prefer_ipv4(addrs))
}

fn connect(addrs: &[SocketAddr], timeout: Duration) -> Result<TcpStream, AtlasError> {
    // Tope por dirección: una IPv6 inalcanzable no debe consumir el timeout
    // entero cuando después hay una IPv4 válida.
    let per_attempt = timeout.min(Duration::from_secs(2));
    for addr in addrs {
        if let Ok(stream) = TcpStream::connect_timeout(addr, per_attempt) {
            return Ok(stream);
        }
    }
    Err(unreachable("No se pudo conectar con el relay."))
}

fn http(
    method: &str,
    base_url: &str,
    path: &str,
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<serde_json::Value, AtlasError> {
    let (host, port) = split_host_port(base_url)?;
    let addrs = resolve(&host, port)?;
    let mut stream = connect(&addrs, timeout)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|_| unreachable("No se pudo configurar el relay."))?;

    let mut request = format!("{method} {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(payload) = body {
        request.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            payload.len()
        ));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|_| unreachable("No se pudo escribir al relay."))?;
    if let Some(payload) = body {
        stream
            .write_all(payload)
            .map_err(|_| unreachable("No se pudo escribir al relay."))?;
    }
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|_| unreachable("El relay cortó la respuesta."))?;

    let text = String::from_utf8_lossy(&raw);
    let (head, body_text) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| invalid("Respuesta del relay sin cabeceras."))?;
    let status: u16 = head
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    if !(200..300).contains(&status) {
        // El relay explica el 400 con `{error: <codigo>}`.
        if status == 400 {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body_text) {
                if parsed.get("error").and_then(|e| e.as_str()) == Some("empty_message") {
                    return Err(AtlasError::new(
                        codes::EMPTY_MESSAGE,
                        "El mensaje está vacío.",
                        false,
                    ));
                }
            }
        }
        return Err(invalid("El relay devolvió un estado inesperado."));
    }
    serde_json::from_str(body_text).map_err(|_| invalid("Cuerpo del relay inválido."))
}

/// Eventos nuevos del cerebro (`seq > since`), convertidos y en orden.
#[derive(Debug, Clone)]
pub struct FetchedEvents {
    pub items: Vec<(u64, OutEvent)>,
    /// Elementos con forma inválida (se omiten sin abortar).
    pub skipped: usize,
}

fn text_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|field| field.as_str())
        .map(|field| field.to_string())
        .filter(|field| !field.is_empty())
}

fn convert(value: &serde_json::Value) -> Option<(u64, OutEvent)> {
    let seq = value.get("seq")?.as_u64()?;
    match value.get("kind")?.as_str()? {
        "message" => {
            let role: Role = serde_json::from_value(value.get("role")?.clone()).ok()?;
            let source: Source = serde_json::from_value(value.get("source")?.clone()).ok()?;
            let content = text_field(value, "content")?;
            Some((
                seq,
                OutEvent::Message(MessageAppended {
                    correlation: Correlation {
                        request_id: text_field(value, "requestId")?,
                        session_id: text_field(value, "sessionId")?,
                        timestamp: text_field(value, "timestamp")?,
                    },
                    kind: EventType::MessageAppended,
                    role,
                    content,
                    source,
                    agent: value
                        .get("agent")
                        .and_then(|field| serde_json::from_value(field.clone()).ok()),
                    task_id: text_field(value, "taskId"),
                }),
            ))
        }
        "state" => {
            let state: SessionState =
                serde_json::from_value(value.get("state")?.clone()).ok()?;
            Some((
                seq,
                OutEvent::Session(SessionEvent {
                    correlation: Correlation {
                        request_id: text_field(value, "requestId")?,
                        session_id: text_field(value, "sessionId")?,
                        timestamp: text_field(value, "timestamp")?,
                    },
                    kind: EventType::SessionStateChanged,
                    state,
                    message: value
                        .get("message")
                        .and_then(|field| field.as_str())
                        .map(|field| field.to_string()),
                }),
            ))
        }
        "confirmation" => {
            let risk: RiskLevel =
                serde_json::from_value(value.get("risk")?.clone()).ok()?;
            Some((
                seq,
                OutEvent::Confirm(ConfirmationRequest {
                    correlation: Correlation {
                        request_id: text_field(value, "requestId")?,
                        session_id: text_field(value, "sessionId")?,
                        timestamp: text_field(value, "timestamp")?,
                    },
                    kind: EventType::PermissionConfirmationRequired,
                    tool_id: text_field(value, "toolId")?,
                    summary: text_field(value, "summary")?,
                    risk,
                    expires_at: text_field(value, "expiresAt")?,
                }),
            ))
        }
        _ => None,
    }
}

/// Lee los eventos nuevos del cerebro.
pub fn get_events(
    base_url: &str,
    since: u64,
    timeout: Duration,
) -> Result<FetchedEvents, AtlasError> {
    let parsed = http(
        "GET",
        base_url,
        &format!("/v1/events?since={since}"),
        None,
        timeout,
    )?;
    let mut items = Vec::new();
    let mut skipped = 0;
    let empty = Vec::new();
    let raw = parsed
        .get("events")
        .and_then(|events| events.as_array())
        .unwrap_or(&empty);
    for item in raw {
        match convert(item) {
            Some(converted) => items.push(converted),
            None => skipped += 1,
        }
    }
    items.sort_by_key(|(seq, _)| *seq);
    Ok(FetchedEvents { items, skipped })
}

/// Envía un mensaje al cerebro. Devuelve `(requestId, sessionId)`.
pub fn post_send(
    base_url: &str,
    content: &str,
    source: &str,
    timeout: Duration,
) -> Result<(String, String), AtlasError> {
    let payload = serde_json::json!({ "content": content, "source": source });
    let parsed = http(
        "POST",
        base_url,
        "/v1/send",
        Some(payload.to_string().as_bytes()),
        timeout,
    )?;
    match (
        text_field(&parsed, "requestId"),
        text_field(&parsed, "sessionId"),
    ) {
        (Some(request_id), Some(session_id)) => Ok((request_id, session_id)),
        _ => Err(invalid("El relay no devolvió identificadores.")),
    }
}

/// Pide al cerebro cancelar una petición (idempotente).
pub fn post_cancel(    base_url: &str,
    request_id: &str,
    timeout: Duration,
) -> Result<(), AtlasError> {
    let payload = serde_json::json!({ "requestId": request_id });
    http(
        "POST",
        base_url,
        "/v1/cancel",
        Some(payload.to_string().as_bytes()),
        timeout,
    )?;
    Ok(())
}

/// Responde a una petición de confirmación. Devuelve si fue aceptada
/// (false también cuando caducó o no existe: idempotente).
pub fn post_confirm(
    base_url: &str,
    request_id: &str,
    decision: &str,
    timeout: Duration,
) -> Result<bool, AtlasError> {
    let payload = serde_json::json!({ "requestId": request_id, "decision": decision });
    let parsed = http(
        "POST",
        base_url,
        "/v1/confirm",
        Some(payload.to_string().as_bytes()),
        timeout,
    )?;
    Ok(parsed
        .get("accepted")
        .and_then(|value| value.as_bool())
        .unwrap_or(false))
}

/// Mensaje guardado en una instantánea de sesión.
#[derive(Debug, Clone, Deserialize)]
pub struct SnapshotMessage {
    #[serde(rename = "requestId")]
    pub request_id: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub role: Role,
    pub content: String,
    pub source: Source,
    pub timestamp: String,
    #[serde(rename = "agent")]
    pub agent: Option<Agent>,
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
}

/// Instantánea de la sesión activa del cerebro (`None` si no hay).
#[derive(Debug, Clone, Deserialize)]
pub struct SessionSnapshot {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub state: SessionState,
    pub messages: Vec<SnapshotMessage>,
}

pub fn get_session(
    base_url: &str,
    timeout: Duration,
) -> Result<(Option<SessionSnapshot>, u64), AtlasError> {
    let parsed = http("GET", base_url, "/v1/session", None, timeout)?;
    let cursor = parsed.get("cursor").and_then(|seq| seq.as_u64()).unwrap_or(0);
    let snapshot = match parsed.get("session") {
        None | Some(serde_json::Value::Null) => None,
        Some(snapshot) => serde_json::from_value(snapshot.clone())
            .map(Some)
            .map_err(|_| invalid("Sesión del relay ilegible."))?,
    };
    Ok((snapshot, cursor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Cerebro falso en localhost: sirve `routes` (path → cuerpo).
    /// Consume la petición entera (cabeceras + cuerpo según Content-Length),
    /// como un servidor HTTP real, antes de responder.
    fn fake_brain(routes: Vec<(&'static str, &'static str)>) -> String {
        use std::io::BufRead;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(routes.len()) {
                let Ok(stream) = stream else {
                    break;
                };
                let mut reader = std::io::BufReader::new(stream);
                let mut first = String::new();
                if reader.read_line(&mut first).unwrap_or(0) == 0 {
                    continue;
                }
                let path = first
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let trimmed = line.trim_end().to_lowercase();
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some(value) = trimmed.strip_prefix("content-length:") {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                }
                if content_length > 0 {
                    let mut body_buf = vec![0u8; content_length];
                    if std::io::Read::read_exact(&mut reader, &mut body_buf).is_err() {
                        continue;
                    }
                }
                let mut stream = reader.into_inner();
                let body = routes
                    .iter()
                    .find(|(route, _)| *route == path)
                    .map(|(_, body)| *body)
                    .unwrap_or(r#"{"ok":false,"error":"not_found"}"#);
                let status = if body.contains("not_found") {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let response = format!(
                    "HTTP/1.0 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn events_convierte_y_omite_invalidos() {
        let url = fake_brain(vec![(
            "/v1/events?since=0",
            r#"{"events": [
              {"seq": 2, "kind": "message", "type": "conversation.message_appended",
               "requestId": "r2", "sessionId": "s", "timestamp": "t",
               "role": "assistant", "content": "hola", "source": "local"},
              {"seq": 1, "kind": "state", "type": "session.state_changed",
               "requestId": "r1", "sessionId": "s", "timestamp": "t",
               "state": "thinking"},
              {"seq": 3, "kind": "message", "role": "user"}
            ]}"#,
        )]);
        let fetched = get_events(&url, 0, Duration::from_secs(5)).expect("fetch ok");
        assert_eq!(fetched.items.len(), 2);
        assert_eq!(fetched.items[0].0, 1);
        assert_eq!(fetched.items[1].0, 2);
        assert_eq!(fetched.skipped, 1);
    }

    #[test]
    fn send_devuelve_ids_y_cancel_no_falla() {
        let url = fake_brain(vec![
            ("/v1/send", r#"{"ok": true, "requestId": "r", "sessionId": "s"}"#),
            ("/v1/cancel", r#"{"ok": true, "cancelled": false}"#),
            ("/v1/confirm", r#"{"ok": true, "accepted": true}"#),
        ]);
        let (request_id, session_id) =
            post_send(&url, "hola", "local", Duration::from_secs(5)).expect("send ok");
        assert_eq!((request_id.as_str(), session_id.as_str()), ("r", "s"));
        post_cancel(&url, "r", Duration::from_secs(5)).expect("cancel ok");
        assert!(post_confirm(&url, "r", "allow_once", Duration::from_secs(5)).expect("confirm ok"));
    }

    #[test]
    fn confirmation_se_convierte() {
        let url = fake_brain(vec![(
            "/v1/events?since=0",
            r#"{"events": [
              {"seq": 1, "kind": "confirmation",
               "type": "permission.confirmation_required",
               "requestId": "r", "sessionId": "s", "timestamp": "t",
               "toolId": "calendar_write", "summary": "Crear X",
               "risk": "write", "expiresAt": "e"}
            ]}"#,
        )]);
        let fetched = get_events(&url, 0, Duration::from_secs(5)).expect("fetch ok");
        assert_eq!(fetched.items.len(), 1);
        assert!(matches!(fetched.items[0].1, OutEvent::Confirm(_)));
    }

    #[test]
    fn ipv4_va_antes_que_ipv6() {
        let addrs = vec![
            "[::1]:8765".parse().unwrap(),
            "192.168.100.26:8765".parse().unwrap(),
        ];
        let ordered = prefer_ipv4(addrs);
        assert!(ordered[0].is_ipv4());
        assert!(ordered[1].is_ipv6());
    }

    #[test]
    fn snapshot_message_lee_agent_y_task() {
        let url = fake_brain(vec![(
            "/v1/session",
            r#"{"session":{"sessionId":"ses-1","state":"idle","messages":[{"requestId":"req-1","sessionId":"ses-1","role":"assistant","content":"eco","source":"local","timestamp":"2026-10-29T00:00:00.000Z","agent":"hefesto","taskId":"tsk-1"}]},"cursor":0}"#,
        )]);
        let (snapshot, _) = get_session(&url, Duration::from_secs(2)).expect("ok");
        let message = &snapshot.expect("sesión").messages[0];
        assert_eq!(message.agent, Some(Agent::Hefesto));
        assert_eq!(message.task_id.as_deref(), Some("tsk-1"));
    }

    #[test]
    fn https_se_rechaza_y_caido_es_reintentable() {
        let err = get_events("https://127.0.0.1:8765", 0, Duration::from_secs(1))
            .expect_err("https no");
        assert_eq!(err.code, codes::RELAY_INVALID);
        let err = get_events("http://127.0.0.1:1", 0, Duration::from_millis(300))
            .expect_err("sin relay");
        assert_eq!(err.code, codes::RELAY_UNREACHABLE);
        assert!(err.retryable);
    }
}
