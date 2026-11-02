//! Puente real de ventanas de Hefesto (Task 4): Accessibility (`AXUIElement`)
//! sobre ApplicationServices y AppKit para resolver apps y pantallas.
//!
//! AX es una API C pública, pero los atributos se piden a la app objetivo y una
//! app colgada puede bloquear la llamada: se fija un timeout de mensajes de 2 s
//! por elemento de aplicación. Igual que los puentes de apps y teclado, todo el
//! acceso a AppKit (`NSWorkspace`, `NSScreen`) y AX se reenvía al hilo
//! principal con `run_on_main_thread`; el hilo de Hefesto espera fuera, con
//! timeout propio.
//!
//! Propiedad de memoria: `AXUIElementCreateApplication`, `AXUIElementCopy-
//! AttributeValue` y `AXValueCreate` devuelven objetos con regla de creación y
//! se envuelven con `wrap_under_create_rule`; los elementos de `AXWindows`
//! siguen la regla get y se retienen con `wrap_under_get_rule` para que la
//! liberación del array no los invalide. En ambos casos el `CFType` resultante
//! hace exactamente un `CFRelease` al caer.
//!
//! Este módulo solo existe en macOS: ApplicationServices y AppKit no están
//! disponibles en el resto de plataformas.

use std::ffi::c_void;
use std::sync::mpsc;
use std::time::Duration;

use atlas_mac::windows::{
    resolve_target, WindowBridge, WindowFailure, WindowInfo, WindowTarget, MAX_WINDOWS,
};
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::string::CFString;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication, NSScreen,
    NSWorkspace,
};
use tauri::AppHandle;

type AXUIElementRef = *const c_void;
type CFTypeRef = *const c_void;

const AX_ERROR_SUCCESS: i32 = 0;
/// `kAXErrorFailure`: valor nulo con código de éxito (no debería ocurrir).
const AX_ERROR_FAILURE: i32 = -25200;
/// `kAXValueCGPointType` / `kAXValueCGSizeType` de `AXValue.h`.
const AX_VALUE_CGPOINT: u32 = 1;
const AX_VALUE_CGSIZE: u32 = 2;
/// Margen mínimo visible de una ventana movida: ancho y barra de título.
const MIN_VISIBLE_WIDTH: i32 = 200;
const MIN_VISIBLE_TITLEBAR: i32 = 40;
const MAIN_ROUNDTRIP: Duration = Duration::from_secs(5);
/// La activación de AppKit es asíncrona: se comprueba el foco real con
/// intentos cortos fuera del hilo principal.
const FRONTMOST_ATTEMPTS: u32 = 12;
const FRONTMOST_PAUSE: Duration = Duration::from_millis(50);
/// Una app colgada no debe retener el hilo principal más de 2 s por mensaje.
const MESSAGING_TIMEOUT: f32 = 2.0;

/// Posición en coordenadas de Accessibility (misma disposición que `CGPoint`).
#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

/// Tamaño en coordenadas de Accessibility (misma disposición que `CGSize`).
#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

// Justificación: son símbolos públicos de ApplicationServices (HIServices),
// documentados por Apple; el bloque solo declara sus firmas, sin estado ni
// símbolos privados.
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFTypeRef,
        value: CFTypeRef,
    ) -> i32;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFTypeRef) -> i32;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
}

const FORWARD_ERROR: &str = "no se pudo reenviar al hilo principal";
const TIMEOUT_ERROR: &str = "timeout del hilo principal";
const NO_ELEMENT_ERROR: &str = "AX sin elemento";
const NO_WINDOW_ERROR: &str = "AX sin ventana";
const NO_VALUE_ERROR: &str = "AX sin valor";

fn ax_failure(code: i32) -> WindowFailure {
    WindowFailure::Detail(format!("AX {code}"))
}

/// Elemento de aplicación creado con regla de creación. Se libera al caer.
struct AppElement(CFType);

impl AppElement {
    fn as_raw(&self) -> AXUIElementRef {
        self.0.as_CFTypeRef()
    }
}

/// Crea el elemento de la app y fija de inmediato el timeout de mensajes AX,
/// antes de cualquier otra petición que pudiera bloquearse.
fn app_element(pid: i32) -> Result<AppElement, WindowFailure> {
    // Justificación: `AXUIElementCreateApplication` es una función C pública
    // con regla de creación; el envoltorio se encarga del único `CFRelease`.
    let raw = unsafe { AXUIElementCreateApplication(pid) };
    if raw.is_null() {
        return Err(WindowFailure::Detail(NO_ELEMENT_ERROR.to_owned()));
    }
    let element = unsafe { CFType::wrap_under_create_rule(raw) };
    // Justificación: API C pública; solo ajusta el timeout de mensajes del
    // elemento. Un fallo deja el default del sistema (la espera externa acota).
    unsafe {
        AXUIElementSetMessagingTimeout(raw, MESSAGING_TIMEOUT);
    }
    Ok(AppElement(element))
}

/// Copia un atributo AX (regla de creación) y lo envuelve para liberarlo una vez.
fn ax_copy(element: AXUIElementRef, attribute: &str) -> Result<CFType, WindowFailure> {
    let attribute = CFString::new(attribute);
    let mut value: CFTypeRef = std::ptr::null();
    // Justificación: copia de atributo de una API C pública; en éxito el valor
    // resultante sigue la regla de creación (se envuelve abajo).
    let code =
        unsafe { AXUIElementCopyAttributeValue(element, attribute.as_CFTypeRef(), &mut value) };
    if code != AX_ERROR_SUCCESS || value.is_null() {
        if !value.is_null() {
            // Justificación: Apple deja `value` indefinido si la copia falla;
            // si aun así devolvió un objeto, tendría regla de creación y se
            // libera al soltarlo para no filtrarlo.
            drop(unsafe { CFType::wrap_under_create_rule(value) });
        }
        let code = if code == AX_ERROR_SUCCESS {
            AX_ERROR_FAILURE
        } else {
            code
        };
        return Err(ax_failure(code));
    }
    // Justificación: valor con regla de creación; el envoltorio lo libera.
    Ok(unsafe { CFType::wrap_under_create_rule(value) })
}

fn ax_string(element: AXUIElementRef, attribute: &str) -> Option<String> {
    let value = ax_copy(element, attribute).ok()?;
    value.downcast::<CFString>().map(|text| text.to_string())
}

fn ax_bool(element: AXUIElementRef, attribute: &str) -> Option<bool> {
    let value = ax_copy(element, attribute).ok()?;
    value.downcast::<CFBoolean>().map(bool::from)
}

fn ax_point(element: AXUIElementRef, attribute: &str) -> Option<(f64, f64)> {
    let value = ax_copy(element, attribute).ok()?;
    let mut point = CGPoint { x: 0.0, y: 0.0 };
    // Justificación: `AXValueGetValue` copia el struct pedido en `point`, que
    // vive en esta función durante toda la llamada.
    let ok = unsafe {
        AXValueGetValue(
            value.as_CFTypeRef(),
            AX_VALUE_CGPOINT,
            &mut point as *mut CGPoint as *mut c_void,
        )
    };
    ok.then_some((point.x, point.y))
}

fn ax_size(element: AXUIElementRef, attribute: &str) -> Option<(f64, f64)> {
    let value = ax_copy(element, attribute).ok()?;
    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    // Justificación: `AXValueGetValue` copia el struct pedido en `size`, que
    // vive en esta función durante toda la llamada.
    let ok = unsafe {
        AXValueGetValue(
            value.as_CFTypeRef(),
            AX_VALUE_CGSIZE,
            &mut size as *mut CGSize as *mut c_void,
        )
    };
    ok.then_some((size.width, size.height))
}

fn set_attribute(
    element: AXUIElementRef,
    attribute: &str,
    value: CFTypeRef,
) -> Result<(), WindowFailure> {
    let attribute = CFString::new(attribute);
    // Justificación: fija un atributo AX con una API C pública; `value` vive en
    // el llamante durante toda la llamada.
    let code = unsafe { AXUIElementSetAttributeValue(element, attribute.as_CFTypeRef(), value) };
    if code == AX_ERROR_SUCCESS {
        Ok(())
    } else {
        Err(ax_failure(code))
    }
}

fn set_bool(element: AXUIElementRef, attribute: &str, value: bool) -> Result<(), WindowFailure> {
    let boolean = CFBoolean::from(value);
    set_attribute(element, attribute, boolean.as_CFTypeRef())
}

/// Geometría que se empaqueta en un `AXValue` (punto o tamaño).
enum AxGeometry {
    Point(CGPoint),
    Size(CGSize),
}

fn set_geometry(
    element: AXUIElementRef,
    attribute: &str,
    geometry: AxGeometry,
) -> Result<(), WindowFailure> {
    let (kind, pointer) = match &geometry {
        AxGeometry::Point(point) => (AX_VALUE_CGPOINT, point as *const CGPoint as *const c_void),
        AxGeometry::Size(size) => (AX_VALUE_CGSIZE, size as *const CGSize as *const c_void),
    };
    // Justificación: el puntero apunta al struct de `geometry`, que vive hasta
    // el final de la función; `AXValueCreate` copia el valor y devuelve un
    // objeto con regla de creación.
    let raw = unsafe { AXValueCreate(kind, pointer) };
    if raw.is_null() {
        return Err(WindowFailure::Detail(NO_VALUE_ERROR.to_owned()));
    }
    let value = unsafe { CFType::wrap_under_create_rule(raw) };
    set_attribute(element, attribute, value.as_CFTypeRef())
}

fn set_point(element: AXUIElementRef, x: i32, y: i32) -> Result<(), WindowFailure> {
    set_geometry(
        element,
        "AXPosition",
        AxGeometry::Point(CGPoint {
            x: f64::from(x),
            y: f64::from(y),
        }),
    )
}

fn set_size(element: AXUIElementRef, width: u32, height: u32) -> Result<(), WindowFailure> {
    set_geometry(
        element,
        "AXSize",
        AxGeometry::Size(CGSize {
            width: f64::from(width),
            height: f64::from(height),
        }),
    )
}

fn perform_action(element: AXUIElementRef, action: &str) -> Result<(), WindowFailure> {
    let action = CFString::new(action);
    // Justificación: ejecuta una acción AX pública sobre la ventana.
    let code = unsafe { AXUIElementPerformAction(element, action.as_CFTypeRef()) };
    if code == AX_ERROR_SUCCESS {
        Ok(())
    } else {
        Err(ax_failure(code))
    }
}

/// Ventanas de la app (`AXWindows`). Cada elemento se retiene para que siga
/// vivo cuando el array de origen se libere al final de la función.
fn ax_windows(element: AXUIElementRef) -> Result<Vec<CFType>, WindowFailure> {
    let value = ax_copy(element, "AXWindows")?;
    let Some(array) = value.downcast_into::<CFArray>() else {
        return Ok(Vec::new());
    };
    Ok(array
        .iter()
        .filter_map(|item| {
            let raw = *item;
            if raw.is_null() {
                None
            } else {
                // Justificación: los elementos de un `CFArray` siguen la regla
                // get; se retienen para sobrevivir a la liberación del array.
                let window = unsafe { CFType::wrap_under_get_rule(raw) };
                // Justificación: el timeout de mensajes de AX es por objeto;
                // sin fijarlo en la ventana, sus lecturas y escrituras usarían
                // el default del sistema y podrían bloquear sin cota.
                unsafe {
                    AXUIElementSetMessagingTimeout(raw, MESSAGING_TIMEOUT);
                }
                Some(window)
            }
        })
        .collect())
}

fn is_standard_window(element: AXUIElementRef) -> bool {
    ax_string(element, "AXRole").as_deref() == Some("AXWindow")
        && ax_string(element, "AXSubrole").as_deref() == Some("AXStandardWindow")
}

/// Datos públicos de una ventana estándar. Si la app no expone posición o
/// tamaño (ventana cerrándose), la ventana se omite en vez de inventarlos.
fn window_info(element: AXUIElementRef, app: &str) -> Option<WindowInfo> {
    let (x, y) = ax_point(element, "AXPosition")?;
    let (width, height) = ax_size(element, "AXSize")?;
    let title = ax_string(element, "AXTitle")
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "Sin título".to_owned());
    Some(WindowInfo {
        app: app.to_owned(),
        title,
        x: to_i32(x),
        y: to_i32(y),
        width: to_u32(width),
        height: to_u32(height),
        focused: ax_bool(element, "AXFocused").unwrap_or(false),
        minimized: ax_bool(element, "AXMinimized").unwrap_or(false),
    })
}

/// Ventanas estándar con su índice dentro del array crudo, para poder operar
/// sobre el mismo `AXUIElement` que se describió. Deja de leer atributos en
/// cuanto ha recogido `limit` ventanas; los listados no necesitan más.
fn standard_windows(windows: &[CFType], app: &str, limit: usize) -> Vec<(WindowInfo, usize)> {
    let mut out = Vec::new();
    for (index, window) in windows.iter().enumerate() {
        if out.len() >= limit {
            break;
        }
        if is_standard_window(window.as_CFTypeRef()) {
            if let Some(info) = window_info(window.as_CFTypeRef(), app) {
                out.push((info, index));
            }
        }
    }
    out
}

fn to_i32(value: f64) -> i32 {
    value.round() as i32
}

fn to_u32(value: f64) -> u32 {
    value.round().max(0.0) as u32
}

/// Busca la app regular por nombre o bundle id (sin distinguir mayúsculas).
/// Solo desde el hilo principal: usa `NSWorkspace`.
fn running_pid(app: &str) -> Result<(i32, String), WindowFailure> {
    let wanted = app.to_lowercase();
    for running in NSWorkspace::sharedWorkspace()
        .runningApplications()
        .to_vec()
    {
        if running.activationPolicy() != NSApplicationActivationPolicy::Regular {
            continue;
        }
        let name = running.localizedName().map(|value| value.to_string());
        let bundle = running.bundleIdentifier().map(|value| value.to_string());
        let matches = name
            .as_deref()
            .is_some_and(|name| name.to_lowercase() == wanted)
            || bundle
                .as_deref()
                .is_some_and(|bundle| bundle.to_lowercase() == wanted);
        if matches {
            let display = name.or(bundle).unwrap_or_else(|| app.to_owned());
            return Ok((running.processIdentifier(), display));
        }
    }
    Err(WindowFailure::Stable(format!("«{app}» no está abierta")))
}

/// Rectángulo que une todas las pantallas en coordenadas AX (origen
/// arriba-izquierda de la pantalla principal). Las pantallas llegan con el
/// origen AppKit (abajo-izquierda) y se convierten con la altura de la
/// principal: `ax_y = main_height - (y + height)`.
fn ax_union(frames: &[(f64, f64, f64, f64)], main_height: f64) -> Option<(f64, f64, f64, f64)> {
    frames
        .iter()
        .map(|&(x, y, width, height)| {
            let top = main_height - (y + height);
            (x, top, x + width, top + height)
        })
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
}

/// Unión de pantallas en coordenadas AX. Solo desde el hilo principal.
fn screen_bounds() -> Option<(f64, f64, f64, f64)> {
    // Si el runtime no nos marca como hilo principal, no se acota a ciegas.
    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm).to_vec();
    // Las coordenadas AX/CG se anclan a la pantalla principal (la del menú),
    // que es la primera de `NSScreen::screens`, no la de la ventana clave.
    let main_height = screens.first()?.frame().size.height;
    let frames: Vec<(f64, f64, f64, f64)> = screens
        .iter()
        .map(|screen| {
            let frame = screen.frame();
            (
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            )
        })
        .collect();
    ax_union(&frames, main_height)
}

/// Deja la barra de título alcanzable: la ventana no puede salir del
/// rectángulo de todas las pantallas, conservando 200 pt de ancho y 40 pt de
/// alto visibles.
fn clamp_position(x: i32, y: i32, bounds: Option<(f64, f64, f64, f64)>) -> (i32, i32) {
    let Some((min_x, min_y, max_x, max_y)) = bounds else {
        return (x, y);
    };
    let high_x = (max_x - f64::from(MIN_VISIBLE_WIDTH)).max(min_x);
    let high_y = (max_y - f64::from(MIN_VISIBLE_TITLEBAR)).max(min_y);
    (
        f64::from(x).clamp(min_x, high_x).round() as i32,
        f64::from(y).clamp(min_y, high_y).round() as i32,
    )
}

/// Ventana ya resuelta, con el PID para poder activar su app.
struct LocatedWindow {
    pid: i32,
    element: CFType,
    info: WindowInfo,
}

/// Resuelve app → ventanas estándar → objetivo. Solo desde el hilo principal.
fn locate(target: &WindowTarget) -> Result<LocatedWindow, WindowFailure> {
    let (pid, name) = running_pid(&target.app)?;
    let app = app_element(pid)?;
    let mut windows = ax_windows(app.as_raw())?;
    // El objetivo puede estar más allá del corte de los listados.
    let standard = standard_windows(&windows, &name, usize::MAX);
    let infos: Vec<WindowInfo> = standard.iter().map(|(info, _)| info.clone()).collect();
    let position = resolve_target(&target.app, &infos, target.title.as_deref(), target.index)?;
    let (info, index) = standard.get(position).cloned().ok_or_else(|| {
        WindowFailure::Stable(format!("no encuentro una ventana de «{}»", target.app))
    })?;
    let element = windows.swap_remove(index);
    Ok(LocatedWindow { pid, element, info })
}

/// Relee la ventana tras una operación para devolver su estado real.
fn refresh(element: &CFType, app: &str) -> Result<WindowInfo, WindowFailure> {
    window_info(element.as_CFTypeRef(), app)
        .ok_or_else(|| WindowFailure::Detail(NO_WINDOW_ERROR.to_owned()))
}

fn list_on_main(filter: Option<&str>) -> Result<Vec<WindowInfo>, WindowFailure> {
    if let Some(app) = filter {
        let (pid, name) = running_pid(app)?;
        let element = app_element(pid)?;
        let windows = ax_windows(element.as_raw())?;
        // Corte temprano: no se leen siete atributos por ventana más allá del
        // máximo que se devuelve.
        return Ok(standard_windows(&windows, &name, MAX_WINDOWS)
            .into_iter()
            .map(|(info, _)| info)
            .take(MAX_WINDOWS)
            .collect());
    }
    let mut out = Vec::new();
    for running in NSWorkspace::sharedWorkspace()
        .runningApplications()
        .to_vec()
    {
        if running.activationPolicy() != NSApplicationActivationPolicy::Regular {
            continue;
        }
        let Some(name) = running
            .localizedName()
            .map(|value| value.to_string())
            .or_else(|| running.bundleIdentifier().map(|value| value.to_string()))
        else {
            continue;
        };
        let Ok(element) = app_element(running.processIdentifier()) else {
            continue;
        };
        let Ok(windows) = ax_windows(element.as_raw()) else {
            continue;
        };
        // Se pide solo lo que falta para el máximo total, sin leer de más.
        let remaining = MAX_WINDOWS.saturating_sub(out.len());
        out.extend(
            standard_windows(&windows, &name, remaining)
                .into_iter()
                .map(|(info, _)| info),
        );
        if out.len() >= MAX_WINDOWS {
            out.truncate(MAX_WINDOWS);
            break;
        }
    }
    Ok(out)
}

fn move_on_main(target: &WindowTarget, x: i32, y: i32) -> Result<WindowInfo, WindowFailure> {
    let located = locate(target)?;
    let (x, y) = clamp_position(x, y, screen_bounds());
    set_point(located.element.as_CFTypeRef(), x, y)?;
    refresh(&located.element, &located.info.app)
}

fn resize_on_main(
    target: &WindowTarget,
    width: u32,
    height: u32,
) -> Result<WindowInfo, WindowFailure> {
    let located = locate(target)?;
    let width = width.clamp(100, 10_000);
    let height = height.clamp(100, 10_000);
    set_size(located.element.as_CFTypeRef(), width, height)?;
    refresh(&located.element, &located.info.app)
}

/// Pide el foco para la app del objetivo (AXFrontmost + activación AppKit) y
/// devuelve su PID. La activación es asíncrona; se confirma aparte.
fn activate_on_main(target: &WindowTarget) -> Result<i32, WindowFailure> {
    let located = locate(target)?;
    let app = app_element(located.pid)?;
    // AXFrontmost es la vía que respeta el sistema aunque la activación de
    // AppKit se rechace; si una app no expone el atributo, se sigue con la
    // activación normal en vez de abortar.
    let _ = set_bool(app.as_raw(), "AXFrontmost", true);
    if let Some(running) =
        NSRunningApplication::runningApplicationWithProcessIdentifier(located.pid)
    {
        running.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
    }
    Ok(located.pid)
}

/// Deja la ventana concreta al frente y devuelve su estado real. Se llama
/// solo cuando la app ya es la de primer plano.
fn raise_on_main(target: &WindowTarget) -> Result<WindowInfo, WindowFailure> {
    let located = locate(target)?;
    let element = located.element.as_CFTypeRef();
    set_bool(element, "AXMinimized", false)?;
    perform_action(element, "AXRaise")?;
    set_bool(element, "AXFocused", true)?;
    refresh(&located.element, &located.info.app)
}

pub struct AxWindows {
    handle: AppHandle,
}

impl AxWindows {
    pub fn new(handle: AppHandle) -> Self {
        Self { handle }
    }

    /// Viaje corto al hilo principal: AX y AppKit se ejecutan allí y el
    /// resultado vuelve por canal. Quien espera es el hilo llamante.
    fn on_main<T, F>(&self, work: F) -> Result<T, WindowFailure>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        self.handle
            .clone()
            .run_on_main_thread(move || {
                let _ = tx.send(work());
            })
            .map_err(|_| WindowFailure::Detail(FORWARD_ERROR.to_owned()))?;
        rx.recv_timeout(MAIN_ROUNDTRIP)
            .map_err(|_| WindowFailure::Detail(TIMEOUT_ERROR.to_owned()))
    }

    /// Espera a que la app sea la de primer plano. Cada consulta cruza al hilo
    /// principal y la espera ocurre en el hilo llamante, para que el runloop
    /// pueda completar la activación asíncrona entre intentos.
    fn wait_frontmost(&self, pid: i32) -> Result<(), WindowFailure> {
        for attempt in 0..FRONTMOST_ATTEMPTS {
            let frontmost = self.on_main(|| {
                NSWorkspace::sharedWorkspace()
                    .frontmostApplication()
                    .map(|app| app.processIdentifier())
            })?;
            if frontmost == Some(pid) {
                return Ok(());
            }
            if attempt + 1 < FRONTMOST_ATTEMPTS {
                std::thread::sleep(FRONTMOST_PAUSE);
            }
        }
        Err(WindowFailure::Detail("la app no aceptó el foco".to_owned()))
    }
}

impl WindowBridge for AxWindows {
    fn permission(&self) -> bool {
        // Justificación: función C pública sin argumentos; solo lee el TCC.
        unsafe { AXIsProcessTrusted() }
    }

    fn list(&self, app: Option<&str>) -> Result<Vec<WindowInfo>, WindowFailure> {
        let filter = app.map(str::to_owned);
        self.on_main(move || list_on_main(filter.as_deref()))?
    }

    fn move_to(&self, target: &WindowTarget, x: i32, y: i32) -> Result<WindowInfo, WindowFailure> {
        let target = target.clone();
        self.on_main(move || move_on_main(&target, x, y))?
    }

    fn resize(
        &self,
        target: &WindowTarget,
        width: u32,
        height: u32,
    ) -> Result<WindowInfo, WindowFailure> {
        let target = target.clone();
        self.on_main(move || resize_on_main(&target, width, height))?
    }

    fn focus(&self, target: &WindowTarget) -> Result<WindowInfo, WindowFailure> {
        let activating = target.clone();
        let pid = self.on_main(move || activate_on_main(&activating))??;
        self.wait_frontmost(pid)?;
        let raising = target.clone();
        self.on_main(move || raise_on_main(&raising))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_union_traduce_de_appkit_a_coordenadas_ax() {
        // Pantalla única de 1920x1080: el origen AX coincide con AppKit.
        assert_eq!(
            ax_union(&[(0.0, 0.0, 1920.0, 1080.0)], 1080.0),
            Some((0.0, 0.0, 1920.0, 1080.0))
        );
        // Segunda pantalla a la derecha.
        assert_eq!(
            ax_union(
                &[(0.0, 0.0, 1920.0, 1080.0), (1920.0, 0.0, 1280.0, 800.0)],
                1080.0
            ),
            Some((0.0, 0.0, 3200.0, 1080.0))
        );
        // Pantalla encima de la principal: en AX tiene y negativa.
        assert_eq!(
            ax_union(
                &[(0.0, 0.0, 1920.0, 1080.0), (0.0, 1080.0, 1920.0, 1080.0)],
                1080.0
            ),
            Some((0.0, -1080.0, 1920.0, 1080.0))
        );
        assert_eq!(ax_union(&[], 1080.0), None);
    }

    #[test]
    fn la_posicion_se_acota_a_la_union_de_pantallas() {
        let bounds = Some((0.0, 0.0, 1920.0, 1080.0));
        assert_eq!(clamp_position(300, 400, bounds), (300, 400));
        assert_eq!(clamp_position(-500, -500, bounds), (0, 0));
        // Se conservan 200 pt de ancho y 40 pt de barra de título.
        assert_eq!(clamp_position(5000, 5000, bounds), (1720, 1040));
        // Con varias pantallas también se admite la zona negativa.
        let wide = Some((-1280.0, -1080.0, 3200.0, 2160.0));
        assert_eq!(clamp_position(-1000, -900, wide), (-1000, -900));
    }

    #[test]
    fn sin_pantallas_no_se_acota() {
        assert_eq!(clamp_position(-3000, 4000, None), (-3000, 4000));
    }

    #[test]
    fn una_pantalla_mas_pequena_que_el_margen_no_invierte_el_rango() {
        let tiny = Some((0.0, 0.0, 100.0, 100.0));
        assert_eq!(clamp_position(5000, 5000, tiny), (0, 60));
    }

    #[test]
    fn las_fracciones_de_retina_se_redondean() {
        assert_eq!(to_i32(12.4), 12);
        assert_eq!(to_i32(12.6), 13);
        assert_eq!(to_i32(-12.6), -13);
        assert_eq!(to_u32(-4.0), 0);
        assert_eq!(to_u32(799.5), 800);
    }
}
