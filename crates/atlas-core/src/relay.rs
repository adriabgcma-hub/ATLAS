//! Cliente mínimo del relay de Atlas (Fase 2).
//!
//! Solo std: `GET /v1/messages?since=N` contra el relay Python en la red local.
//! Sin dependencias HTTP para mantener el núcleo ligero. Solo `http://`:
//! el relay nunca se expone a internet (polling saliente, bind LAN).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::Deserialize;

use super::contracts::{AtlasError, Source, codes};

/// Mensaje tal cual lo sirve el relay.
#[derive(Debug, Clone, Deserialize)]
pub struct RelayMessage {
    pub seq: u64,
    pub content: String,
    pub source: Source,
    #[serde(default)]
    pub ts: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessagesBody {
    messages: Vec<serde_json::Value>,
}

/// Resultado de una lectura del relay.
#[derive(Debug, Clone)]
pub struct FetchResult {
    pub messages: Vec<RelayMessage>,
    /// Elementos con forma inválida (se omiten sin abortar).
    pub skipped: usize,
}

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

/// Lee los mensajes nuevos del relay (`seq > since`), en orden.
pub fn fetch_messages(
    base_url: &str,
    since: u64,
    timeout: Duration,
) -> Result<FetchResult, AtlasError> {
    let (host, port) = split_host_port(base_url)?;
    let addr = format!("{host}:{port}")
        .to_socket_addrs()
        .map_err(|_| unreachable("No se resuelve el relay."))
        .and_then(|mut it| it.next().ok_or_else(|| unreachable("Relay sin dirección.")))?;

    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|_| unreachable("No se pudo conectar con el relay."))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|_| unreachable("No se pudo configurar el relay."))?;

    let request = format!(
        "GET /v1/messages?since={since} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| unreachable("No se pudo pedir mensajes al relay."))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|_| unreachable("El relay cortó la respuesta."))?;

    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| invalid("Respuesta del relay sin cabeceras."))?;
    let status = head.lines().next().unwrap_or("");
    if !(status.contains(" 200 ") || status.ends_with(" 200")) {
        return Err(invalid("El relay devolvió un estado inesperado."));
    }
    let parsed: MessagesBody =
        serde_json::from_str(body).map_err(|_| invalid("Cuerpo del relay inválido."))?;

    let mut messages = Vec::new();
    let mut skipped = 0;
    for item in parsed.messages {
        match serde_json::from_value::<RelayMessage>(item) {
            Ok(message) => messages.push(message),
            Err(_) => skipped += 1,
        }
    }
    messages.sort_by_key(|m| m.seq);
    Ok(FetchResult { messages, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    /// Relay falso en localhost: responde una vez con `body`.
    fn fake_relay(body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let owned = body.to_string();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    owned.len(),
                    owned
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn fetch_ordena_y_omite_invalidos() {
        let url = fake_relay(
            r#"{"messages":[
              {"seq": 3, "content": "tercero", "source": "telegram"},
              {"seq": 1, "content": "  ", "source": "local"},
              {"seq": 2, "content": "segundo", "source": "sms"}
            ]}"#,
        );
        let result = fetch_messages(&url, 0, Duration::from_secs(5)).expect("fetch ok");
        // seq 1 llega (el núcleo lo omitirá por vacío); seq 2 se omite (origen malo).
        assert_eq!(result.messages.len(), 2);
        assert_eq!(result.messages[0].seq, 1);
        assert_eq!(result.messages[1].seq, 3);
        assert_eq!(result.skipped, 1);
    }

    #[test]
    fn fetch_a_puerto_cerrado_es_reintentable() {
        let err = fetch_messages("http://127.0.0.1:1", 0, Duration::from_millis(300))
            .expect_err("sin relay");
        assert_eq!(err.code, codes::RELAY_UNREACHABLE);
        assert!(err.retryable);
    }

    #[test]
    fn https_se_rechaza() {
        let err = fetch_messages("https://127.0.0.1:8765", 0, Duration::from_secs(1))
            .expect_err("https no");
        assert_eq!(err.code, codes::RELAY_INVALID);
    }
}
