//! ATLAS Panel: shell Tauri v2 independiente de la app principal.
//!
//! F1: solo transporte HTTP hacia el relay. La UI (React compartida con
//! `apps/panel`) llama a `api_get`/`api_post` mediante `invoke`, así que no
//! hay CORS ni peticiones directas del webview al relay.
//!
//! Seguridad: el webview nunca elige la URL del relay, solo la ruta; el
//! backend valida la ruta (`/v1/...`) y añade la base fija. El reinicio del
//! relay (SSH con clave restringida) llega en F3 y será un comando fijo.

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

/// Base del relay en la LAN. No es un secreto. Por nombre (.local): la IP
/// del servidor puede cambiar, el nombre no.
const DEFAULT_RELAY_URL: &str = "http://atlas-hub.local:8765";

fn relay_url() -> String {
    std::env::var("ATLAS_RELAY_URL").unwrap_or_else(|_| DEFAULT_RELAY_URL.to_string())
}

fn client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(ready) = CLIENT.get() {
        return Ok(ready);
    }
    let built = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("cliente HTTP: {err}"))?;
    let _ = CLIENT.set(built);
    CLIENT
        .get()
        .ok_or_else(|| "cliente HTTP no inicializado".to_string())
}

/// Solo rutas de la API del relay. Rechaza URLs absolutas, `..` y espacios
/// para que el webview no pueda convertir el comando en un proxy genérico.
fn validate_path(path: &str) -> Result<&str, String> {
    let path = path.trim();
    if !path.starts_with("/v1/") {
        return Err("ruta no permitida: debe empezar por /v1/".to_string());
    }
    if path.contains("..")
        || path.contains("://")
        || path.contains('#')
        || path.chars().any(char::is_whitespace)
    {
        return Err("ruta no válida".to_string());
    }
    Ok(path)
}

async fn request(
    method: reqwest::Method,
    path: String,
    body: Option<Value>,
) -> Result<Value, String> {
    let path = validate_path(&path)?;
    #[cfg(debug_assertions)]
    eprintln!("ATLAS Panel → {method} {path}");
    let url = format!("{}{}", relay_url(), path);
    let mut req = client()?
        .request(method, &url)
        .header(reqwest::header::ACCEPT, "application/json");
    if let Some(payload) = body {
        req = req.json(&payload);
    }

    let res = req
        .send()
        .await
        .map_err(|err| format!("sin conexión con el relay: {err}"))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|err| format!("respuesta ilegible: {err}"))?;

    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status.as_u16(), text));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|err| format!("JSON no válido: {err}"))
}

#[tauri::command]
async fn api_get(path: String) -> Result<Value, String> {
    request(reqwest::Method::GET, path, None).await
}

#[tauri::command]
async fn api_post(path: String, body: Value) -> Result<Value, String> {
    request(reqwest::Method::POST, path, Some(body)).await
}

fn main() {
    let code = match tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![api_get, api_post])
        .run(tauri::generate_context!())
    {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("ATLAS Panel no pudo arrancar: {err}");
            1
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::validate_path;

    #[test]
    fn acepta_rutas_de_la_api() {
        assert_eq!(validate_path("/v1/health").unwrap(), "/v1/health");
        assert_eq!(
            validate_path("  /v1/system/asks  ").unwrap(),
            "/v1/system/asks"
        );
        assert_eq!(
            validate_path("/v1/logs?tail=140").unwrap(),
            "/v1/logs?tail=140"
        );
    }

    #[test]
    fn rechaza_urls_absolutas_y_travesia() {
        for bad in [
            "",
            "/",
            "/panel/",
            "/v2/health",
            "http://evil.example/v1/health",
            "//evil.example/v1/health",
            "/v1/../secreto",
            "/v1/health/../../etc/passwd",
            "/v1/health con espacios",
            "/v1/health#frag",
        ] {
            assert!(validate_path(bad).is_err(), "debería rechazar {bad:?}");
        }
    }

    /// Prueba de humo real contra el relay de la LAN. Ignorada por defecto
    /// para que la suite no dependa de la red:
    /// `cargo test -p atlas-panel -- --ignored`
    #[tokio::test]
    #[ignore = "requiere el relay accesible en la LAN"]
    async fn consulta_health_del_relay_real() {
        let value = super::request(reqwest::Method::GET, "/v1/health".into(), None)
            .await
            .expect("el relay debería responder");
        assert_eq!(
            value.get("ok").and_then(|v| v.as_bool()),
            Some(true),
            "respuesta inesperada: {value}"
        );
    }
}
