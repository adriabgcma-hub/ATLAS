//! ATLAS Panel: app macOS independiente de la app principal.
//!
//! F1: transporte HTTP hacia el relay. La UI (React compartida con
//! `apps/panel`) llama a `api_get`/`api_post` mediante `invoke`, así que no
//! hay CORS ni peticiones directas del webview al relay.
//!
//! F2: icono de barra de menús con semáforo, sondeo de `/v1/health` cada 10 s
//! y ventana que se oculta al cerrar (la app sigue viva en la barra).
//!
//! Seguridad: el webview nunca elige la URL del relay, solo la ruta; el
//! backend valida la ruta (`/v1/...`) y añade la base fija. El reinicio del
//! relay (SSH con clave restringida) llega en F3 y será un comando fijo.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;
#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

/// Base del relay en la LAN. No es un secreto. Por nombre (.local): la IP
/// del servidor puede cambiar, el nombre no.
const DEFAULT_RELAY_URL: &str = "http://atlas-hub.local:8765";

const HEALTH_INTERVAL: Duration = Duration::from_secs(10);

/// Estado compartido mínimo: último estado conocido del relay (para rehacer
/// el menú al cambiar el autoarranque) y si el autoarranque está disponible.
struct PanelState {
    status: Mutex<RelayStatus>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RelayStatus {
    Ok,
    Degraded,
    Down,
}

impl RelayStatus {
    fn label(self) -> &'static str {
        match self {
            RelayStatus::Ok => "OK",
            RelayStatus::Degraded => "degradado",
            RelayStatus::Down => "sin conexión",
        }
    }

    fn menu_label(self) -> &'static str {
        match self {
            RelayStatus::Ok => "🟢 Relay: OK",
            RelayStatus::Degraded => "🟠 Relay: degradado",
            RelayStatus::Down => "🔴 Relay: sin conexión",
        }
    }

    fn icon(self) -> Image<'static> {
        // Iconos embebidos en el binario: no dependen de archivos externos.
        let bytes: &'static [u8] = match self {
            RelayStatus::Ok => include_bytes!("../icons/tray-ok.png"),
            RelayStatus::Degraded => include_bytes!("../icons/tray-warn.png"),
            RelayStatus::Down => include_bytes!("../icons/tray-down.png"),
        };
        Image::from_bytes(bytes).expect("icono de bandeja embebido no válido")
    }
}

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

/// Estado del relay para el semáforo. Un HTTP no-2xx o un JSON sin `ok:true`
/// cuenta como degradado (el relay responde, pero mal); un fallo de red o de
/// timeout cuenta como caído.
async fn fetch_health() -> RelayStatus {
    let Ok(http) = client() else {
        return RelayStatus::Down;
    };
    let url = format!("{}/v1/health", relay_url());
    let res = match http
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(res) => res,
        Err(_) => return RelayStatus::Down,
    };
    if !res.status().is_success() {
        return RelayStatus::Degraded;
    }
    let Ok(payload) = res.json::<Value>().await else {
        return RelayStatus::Degraded;
    };
    if payload.get("ok").and_then(Value::as_bool) == Some(true) {
        RelayStatus::Ok
    } else {
        RelayStatus::Degraded
    }
}

fn build_menu(app: &AppHandle, status: RelayStatus) -> tauri::Result<Menu<tauri::Wry>> {
    let state = MenuItem::with_id(app, "status", status.menu_label(), false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Abrir panel", true, None::<&str>)?;
    // El reinicio real llega en F3 (SSH con clave restringida).
    let restart = MenuItem::with_id(app, "restart", "Reiniciar relay…", false, None::<&str>)?;
    let autostart_enabled = !tauri::is_dev() && app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "Abrir al iniciar sesión",
        !tauri::is_dev(),
        autostart_enabled,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Salir", true, Some("CmdOrCtrl+Q"))?;
    Menu::with_items(
        app,
        &[
            &state,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &restart,
            &PredefinedMenuItem::separator(app)?,
            &autostart,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )
}

fn show_panel(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn current_status(app: &AppHandle) -> RelayStatus {
    app.try_state::<PanelState>()
        .and_then(|state| state.status.lock().ok().map(|guard| *guard))
        .unwrap_or(RelayStatus::Down)
}

/// Autoarranque al iniciar sesión mediante LaunchAgent. En dev no se toca
/// para no registrar el binario de `target/debug` en el sistema.
fn init_autostart(app: &AppHandle) {
    if tauri::is_dev() {
        return;
    }
    let manager = app.autolaunch();
    if manager.is_enabled().unwrap_or(false) {
        return;
    }
    let Ok(dir) = app.path().app_config_dir() else {
        return;
    };
    let marker = dir.join("autostart-initialized");
    // Si el usuario lo desactivó desde el menú, el marcador existe: se respeta
    // y no se vuelve a activar en cada arranque.
    if marker.exists() {
        return;
    }
    match manager.enable() {
        Ok(()) => {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&marker, b"1");
        }
        Err(err) => eprintln!("ATLAS Panel · no se pudo activar el autoarranque: {err}"),
    }
}

fn toggle_autostart(app: &AppHandle) {
    if tauri::is_dev() {
        return;
    }
    let manager = app.autolaunch();
    let enabled = manager.is_enabled().unwrap_or(false);
    let result = if enabled {
        manager.disable()
    } else {
        manager.enable()
    };
    if let Err(err) = result {
        eprintln!("ATLAS Panel · autoarranque: {err}");
    }
    apply_status(app, current_status(app));
}

fn apply_status(app: &AppHandle, status: RelayStatus) {
    if let Some(state) = app.try_state::<PanelState>() {
        if let Ok(mut current) = state.status.lock() {
            *current = status;
        }
    }
    #[cfg(debug_assertions)]
    eprintln!("ATLAS Panel · relay {}", status.label());
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon(Some(status.icon()));
        let _ = tray.set_tooltip(Some(format!("ATLAS Panel · relay {}", status.label())));
        if let Ok(menu) = build_menu(app, status) {
            let _ = tray.set_menu(Some(menu));
        }
    }
    // Disponible para la UI (banner/avisos en fases siguientes).
    let _ = app.emit("panel://relay-status", status.label());
}

fn spawn_health_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last: Option<RelayStatus> = None;
        loop {
            let status = fetch_health().await;
            if last != Some(status) {
                apply_status(&app, status);
                last = Some(status);
            }
            tokio::time::sleep(HEALTH_INTERVAL).await;
        }
    });
}

fn main() {
    let code = match tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(PanelState {
            status: Mutex::new(RelayStatus::Down),
        })
        .invoke_handler(tauri::generate_handler![api_get, api_post])
        .setup(|app| {
            // App de barra de menús: sin icono en el Dock.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(ActivationPolicy::Accessory);

            // Primer arranque del bundle: activar el autoarranque al login.
            init_autostart(app.handle());

            let menu = build_menu(app.handle(), RelayStatus::Down)?;
            TrayIconBuilder::with_id("main")
                .icon(RelayStatus::Down.icon())
                .tooltip("ATLAS Panel · relay sin conexión")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_panel(app),
                    "autostart" => toggle_autostart(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_panel(tray.app_handle());
                    }
                })
                .build(app)?;

            spawn_health_loop(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Cerrar la ventana no cierra la app: queda en la barra de menús.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
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
