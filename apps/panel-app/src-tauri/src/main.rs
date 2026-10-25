//! Hub Panel: app macOS independiente de la app principal.
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
use tauri_plugin_notification::NotificationExt;

/// Base del relay en la LAN. No es un secreto. Por nombre (.local): la IP
/// del servidor puede cambiar, el nombre no.
const DEFAULT_RELAY_URL: &str = "http://atlas-hub.local:8765";

const HEALTH_INTERVAL: Duration = Duration::from_secs(10);
/// Aviso de caída: una notificación por caída, con este margen entre avisos
/// para que un relay que oscila no genere una ráfaga.
const NOTIFY_COOLDOWN: Duration = Duration::from_secs(300);

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
    eprintln!("Hub Panel → {method} {path}");
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

/// Host SSH fijo del relay. El comando remoto real vive en `authorized_keys`
/// (comando forzado); aquí no se acepta nada del webview.
const PANEL_SSH_HOST: &str = "atlas-hub";
const SSH_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, serde::Serialize)]
struct RestartResult {
    ok: bool,
    state: String,
    output: String,
}

fn ssh_key_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join("keys").join("panel_ed25519"))
        .map_err(|err| format!("no se pudo resolver Application Support: {err}"))
}

/// Reinicia `atlas-relay.service` por SSH con la clave restringida del panel.
///
/// La clave instalada en `authorized_keys` tiene un comando forzado que solo
/// ejecuta `sudo systemctl restart atlas-relay && sudo systemctl is-active
/// atlas-relay`; este comando no puede pedir otra cosa. Se envía un argumento
/// inofensivo porque el comando forzado lo ignora, y si la entrada no
/// estuviera bien configurada, `ssh` falla en vez de abrir una shell.
#[tauri::command]
async fn restart_relay(app: AppHandle) -> Result<RestartResult, String> {
    let key = ssh_key_path(&app)?;
    run_restart(&key).await
}

/// Ejecuta el reinicio con la clave indicada. Separado del comando Tauri para
/// poder probarlo de extremo a extremo con `cargo test -- --ignored`.
async fn run_restart(key: &std::path::Path) -> Result<RestartResult, String> {
    if !key.exists() {
        return Err(format!(
            "falta la clave SSH del panel en {}; instala la clave restringida (ver docs/panel-app-plan.md)",
            key.display()
        ));
    }

    let mut command = tokio::process::Command::new("/usr/bin/ssh");
    command
        .arg("-i")
        .arg(key)
        .arg("-o")
        .arg("IdentitiesOnly=yes")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=8")
        .arg("-o")
        .arg("StrictHostKeyChecking=yes")
        .arg(PANEL_SSH_HOST)
        .arg("panel-restart")
        .kill_on_drop(true);

    let output = tokio::time::timeout(SSH_TIMEOUT, command.output())
        .await
        .map_err(|_| "tiempo de espera agotado al reiniciar el relay".to_string())?
        .map_err(|err| format!("no se pudo ejecutar ssh: {err}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let detail = if stderr.is_empty() { &stdout } else { &stderr };
        return Err(format!(
            "el reinicio falló (código {}): {}",
            output.status.code().unwrap_or(-1),
            detail
        ));
    }

    let state = stdout.lines().last().unwrap_or("").trim().to_string();
    let ok = state == "active";
    Ok(RestartResult {
        ok,
        state,
        output: stdout,
    })
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
    // El reinicio real se ejecuta por SSH con la clave restringida (F3).
    let restart = MenuItem::with_id(app, "restart", "Reiniciar relay…", true, None::<&str>)?;
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
    #[cfg(target_os = "macos")]
    set_window_policy(app, true);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Con la ventana visible la app se comporta como una app normal (menú
/// superior y Cmd+Tab); al ocultarla vuelve a ser solo icono de barra, sin
/// Dock. `Accessory` solo no tiene menú ni aparece en Cmd+Tab: por eso se
/// alterna según haya ventana o no.
#[cfg(target_os = "macos")]
fn set_window_policy(app: &AppHandle, visible: bool) {
    let policy = if visible {
        ActivationPolicy::Regular
    } else {
        ActivationPolicy::Accessory
    };
    if let Err(err) = app.set_activation_policy(policy) {
        eprintln!("Hub Panel · política de activación: {err}");
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
        Err(err) => eprintln!("Hub Panel · no se pudo activar el autoarranque: {err}"),
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
        eprintln!("Hub Panel · autoarranque: {err}");
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
    eprintln!("Hub Panel · relay {}", status.label());
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon(Some(status.icon()));
        let _ = tray.set_tooltip(Some(format!("Hub Panel · relay {}", status.label())));
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
        let mut last_notice: Option<std::time::Instant> = None;
        loop {
            let status = fetch_health().await;
            if last != Some(status) {
                apply_status(&app, status);
                // Solo al entrar en "sin conexión" (nunca en cada sondeo) y con
                // margen entre avisos. En dev se silencia para no molestar en
                // cada recarga de `tauri dev`; `ATLAS_PANEL_NOTIFY=1` lo fuerza
                // para poder probar el aviso sin montar el bundle.
                let due = last_notice.is_none_or(|at| at.elapsed() >= NOTIFY_COOLDOWN);
                if status == RelayStatus::Down && notifications_enabled() && due {
                    notify_relay_down(&app);
                    last_notice = Some(std::time::Instant::now());
                }
                last = Some(status);
            }
            tokio::time::sleep(HEALTH_INTERVAL).await;
        }
    });
}

/// Las notificaciones se silencian en dev (cada recarga de `tauri dev`
/// dispararía avisos); `ATLAS_PANEL_NOTIFY=1` permite probarlas sin bundle.
fn notifications_enabled() -> bool {
    !tauri::is_dev() || std::env::var("ATLAS_PANEL_NOTIFY").is_ok()
}

/// Notificación macOS al caer el relay. Un clic abre la ventana (la
/// confirmación del reinicio sigue viviendo en la UI).
fn notify_relay_down(app: &AppHandle) {
    let body = format!(
        "No se puede alcanzar {}. Abre el panel para reiniciarlo.",
        relay_url()
    );
    let result = app
        .notification()
        .builder()
        .title("Hub Panel · relay sin conexión")
        .body(body)
        .show();
    if let Err(err) = result {
        eprintln!("Hub Panel · no se pudo mostrar la notificación: {err}");
    }
}

fn init_notifications(app: &AppHandle) {
    let handle = app.clone();
    if let Err(err) = app.notification().on_action(move |action| {
        if action.action_id() == "tap" {
            show_panel(&handle);
        }
    }) {
        eprintln!("Hub Panel · notificaciones: {err}");
    }
}

fn main() {
    let code = match tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .manage(PanelState {
            status: Mutex::new(RelayStatus::Down),
        })
        .invoke_handler(tauri::generate_handler![api_get, api_post, restart_relay])
        .setup(|app| {
            // Con la ventana abierta al arrancar: app normal (Dock, Cmd+Tab y
            // menú superior); al cerrarla pasará a solo icono de barra.
            #[cfg(target_os = "macos")]
            set_window_policy(app.handle(), true);

            // Primer arranque del bundle: activar el autoarranque al login.
            init_autostart(app.handle());

            // Aviso de caída: registrar el clic antes de que el bucle emita el
            // primero (el plugin descarta acciones anteriores al handler).
            init_notifications(app.handle());

            let menu = build_menu(app.handle(), RelayStatus::Down)?;
            TrayIconBuilder::with_id("main")
                .icon(RelayStatus::Down.icon())
                .tooltip("Hub Panel · relay sin conexión")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_panel(app),
                    "restart" => {
                        // La confirmación doble vive en la UI; el menú solo la abre.
                        show_panel(app);
                        let _ = app.emit("panel://restart-requested", ());
                    }
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
            // Cerrar la ventana no cierra la app: queda en la barra de menús
            // (y deja de aparecer en Dock/Cmd+Tab hasta que se reabra).
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                #[cfg(target_os = "macos")]
                set_window_policy(window.app_handle(), false);
            }
        })
        .run(tauri::generate_context!())
    {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("Hub Panel no pudo arrancar: {err}");
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

    /// Reinicio real del relay con la clave restringida del panel. Requiere
    /// haber instalado la clave en `authorized_keys` (F3).
    /// `cargo test -p atlas-panel -- --ignored`
    #[tokio::test]
    #[ignore = "reinicia atlas-relay y requiere la clave del panel instalada"]
    async fn reinicia_el_relay_real() {
        let home = std::env::var("HOME").expect("HOME");
        let key = std::path::PathBuf::from(home)
            .join("Library/Application Support/com.atlas.panel/keys/panel_ed25519");
        let result = super::run_restart(&key)
            .await
            .expect("el reinicio debería funcionar");
        assert!(result.ok, "el relay no quedó activo: {result:?}");
        assert_eq!(result.state, "active");
    }
}
