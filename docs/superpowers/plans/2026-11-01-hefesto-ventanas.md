# Ventanas de Hefesto Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hefesto lista, mueve, redimensiona y trae al frente ventanas de apps en ejecución mediante la API pública de Accesibilidad, con protocolo v7, tarjeta en los efectos y sin capturas.

**Architecture:** Una herramienta de dispositivo `hefesto.windows` (`list_windows|move_window|resize_window|focus_window`) y una de relay `hefesto_windows` (`list|move|resize|focus`). La lógica pura (validación, resolución de objetivo, textos) vive en `crates/atlas-mac/src/windows.rs` detrás del trait `WindowBridge`; `src-tauri/src/hefesto_windows.rs` lo implementa con `AXUIElement` (ApplicationServices) y AppKit en el hilo principal, sin dependencias nuevas.

**Tech Stack:** Rust (atlas-mac, Tauri, FFI a ApplicationServices + core-foundation 0.10 ya presente), Python (journal/relay), React (órbita).

**Spec:** `docs/superpowers/specs/2026-11-01-hefesto-ventanas-design.md`

## Global Constraints

- `protocolVersion` sube de 6 a 7; ambos extremos rechazan otras versiones.
- Acciones del wire: `list_windows`, `move_window`, `resize_window`, `focus_window` (distintas de `hefesto.apps` para no colisionar en el enum untagged). El relay traduce `list|move|resize|focus`.
- Solo ventanas estándar (`AXWindow` + `AXStandardWindow`); `list` ≤30; `list` sin tarjeta; efectos con **una** tarjeta por llamada válida y **ninguna** con entrada inválida. Sin burbuja `agent: hefesto`.
- Textos estables exactos (copiar tal cual, con `–` U+2013 en los rangos):
  - `action inválida (list, move, resize o focus)`
  - `falta app`
  - `index fuera de rango (1–99)`
  - `x/y fuera de rango (-20000–20000)`
  - `width/height fuera de rango (100–10000)`
  - `falta el permiso de Accesibilidad; actívalo en Ajustes del Sistema › Privacidad y seguridad › Accesibilidad`
  - `«X» no está abierta`
  - `no encuentro una ventana de «X»`
  - `hay N ventanas de «X» que coinciden; concreta el título o el índice`
  - `no se pudo {mover|redimensionar|traer} la ventana: {err}`
  - `Hefesto no puede gestionar ventanas` (sin puente)
- Sin AppleScript, shell, API privada ni capturas. Sin permisos nuevos: `AXIsProcessTrusted` reutiliza Accesibilidad.
- Los títulos de ventana no se registran en logs ni en burbujas (solo en la submission y la tarjeta).

---

### Task 1: Protocolo v6 → v7

**Files:**
- Modify: `crates/atlas-mac/src/wire.rs`, `crates/atlas-mac/tests/wire.rs`, `crates/atlas-mac/src/client.rs`
- Modify: `server/hefesto.py`, `server/atlas_relay.py`, `server/test_hefesto.py`, `server/test_hefesto_delegation.py`

**Interfaces:**
- Produces: `ProtocolVersion` serializa/acepta únicamente `7`.

- [ ] **Step 1: Reemplazar literales**

```bash
sed -i '' 's/"protocolVersion":6/"protocolVersion":7/g; s/"protocolVersion": 6/"protocolVersion": 7/g' \
  crates/atlas-mac/tests/wire.rs crates/atlas-mac/src/client.rs \
  server/hefesto.py server/atlas_relay.py server/test_hefesto.py server/test_hefesto_delegation.py
```

- [ ] **Step 2: Ajustes manuales**

En `crates/atlas-mac/src/wire.rs`: `if value == 6` → `== 7`; `From<ProtocolVersion>` devuelve 7; test renombrado `protocol_version_is_seven`:

```rust
    #[test]
    fn protocol_version_is_seven() {
        assert_eq!(u64::from(ProtocolVersion), 7);
        assert!(ProtocolVersion::try_from(7u64).is_ok());
        assert!(ProtocolVersion::try_from(6u64).is_err());
    }
```

En `server/hefesto.py`: `body["protocolVersion"] != 6` → `!= 7`.
En `server/test_hefesto.py`: `self.assertEqual(value["protocolVersion"], 6)` → `7`.
En `crates/atlas-mac/tests/wire.rs`: renombrar `teclado_v6_...`/`media_v6_...` a `..._v7_...` y ajustar el `.replace("\"protocolVersion\":6", ...)` a 7.

- [ ] **Step 3: Suites**

Run: `cargo test -p atlas-mac` y `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(hefesto): protocolo v7 para ventanas"
```

---

### Task 2: Wire de ventanas y módulo puro

**Files:**
- Modify: `crates/atlas-mac/src/wire.rs`, `crates/atlas-mac/tests/wire.rs`
- Create: `crates/atlas-mac/src/windows.rs`
- Modify: `crates/atlas-mac/src/lib.rs`

**Interfaces:**
- Produces:
  - `wire::WindowAction { ListWindows, MoveWindow, ResizeWindow, FocusWindow }` (serde snake_case)
  - `wire::WindowsPayload { action, app, title, index, x, y, width, height }`
  - `wire::WindowRecord`, `wire::WindowsOutput`, `OutputPayload::Windows`
  - `wire::ToolId::Windows`, `ToolArguments::Windows`
  - `windows::WindowTarget`, `windows::WindowInfo`, `windows::WindowFailure { Stable(String), Detail(String) }`
  - `windows::WindowBridge { permission, list, move_to, resize, focus }`
  - `windows::resolve_target`, `windows::handle_windows`

- [ ] **Step 1: Añadir wire (test primero)**

En `crates/atlas-mac/tests/wire.rs`:

```rust
#[test]
fn ventanas_v7_aterrizan_en_su_variante_sin_robar_apps() {
    let list: ToolArguments = serde_json::from_str(r#"{"action":"list_windows"}"#).unwrap();
    assert!(matches!(list, ToolArguments::Windows(_)));
    let move_it: ToolArguments =
        serde_json::from_str(r#"{"action":"move_window","app":"TextEdit","x":10,"y":20}"#)
            .unwrap();
    assert_eq!(
        move_it,
        ToolArguments::Windows(WindowsPayload {
            action: WindowAction::MoveWindow,
            app: Some("TextEdit".to_owned()),
            title: None,
            index: None,
            x: Some(10),
            y: Some(20),
            width: None,
            height: None,
        })
    );
    let apps: ToolArguments = serde_json::from_str(r#"{"action":"list"}"#).unwrap();
    assert!(matches!(apps, ToolArguments::Apps(_)));
}
```

En `crates/atlas-mac/src/wire.rs`, tras `MediaPayload`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowAction {
    ListWindows,
    MoveWindow,
    ResizeWindow,
    FocusWindow,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowsPayload {
    pub action: WindowAction,
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub index: Option<u8>,
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowRecord {
    pub app: String,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
    pub minimized: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowsOutput {
    pub windows: Vec<WindowRecord>,
}
```

Añadir `Windows(WindowsPayload)` al final de `ToolArguments`, `Windows` a `ToolId` (`"hefesto.windows"`), `Windows(WindowsOutput)` a `OutputPayload`, y ajustar `text_value` para devolver `None` en `Windows`.

- [ ] **Step 2: Crear `crates/atlas-mac/src/windows.rs`**

```rust
use crate::keyboard::NO_PERMISSION_TEXT;
use crate::wire::{
    OutputPayload, WindowAction, WindowRecord, WindowsOutput, WindowsPayload,
};

pub const MISSING_APP_TEXT: &str = "falta app";
pub const BAD_INDEX_TEXT: &str = "index fuera de rango (1–99)";
pub const BAD_POSITION_TEXT: &str = "x/y fuera de rango (-20000–20000)";
pub const BAD_SIZE_TEXT: &str = "width/height fuera de rango (100–10000)";
pub const MAX_WINDOWS: usize = 30;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowTarget {
    pub app: String,
    pub title: Option<String>,
    pub index: Option<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowInfo {
    pub app: String,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
    pub minimized: bool,
}

impl WindowInfo {
    pub fn record(&self) -> WindowRecord {
        WindowRecord {
            app: self.app.clone(),
            title: self.title.clone(),
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            focused: self.focused,
            minimized: self.minimized,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WindowFailure {
    /// Mensaje final ya redactado del puente (`«X» no está abierta`, ambigüedad…).
    Stable(String),
    /// Detalle que el módulo envuelve con «no se pudo … la ventana: …».
    Detail(String),
}

pub trait WindowBridge: Send + Sync {
    fn permission(&self) -> bool;
    fn list(&self, app: Option<&str>) -> Result<Vec<WindowInfo>, WindowFailure>;
    fn move_to(&self, target: &WindowTarget, x: i32, y: i32)
        -> Result<WindowInfo, WindowFailure>;
    fn resize(&self, target: &WindowTarget, width: u32, height: u32)
        -> Result<WindowInfo, WindowFailure>;
    fn focus(&self, target: &WindowTarget) -> Result<WindowInfo, WindowFailure>;
}

fn build_target(payload: &WindowsPayload) -> Result<WindowTarget, &'static str> {
    let app = payload.app.as_deref().map(str::trim).unwrap_or("");
    if app.is_empty() {
        return Err(MISSING_APP_TEXT);
    }
    if let Some(index) = payload.index {
        if !(1..=99).contains(&index) {
            return Err(BAD_INDEX_TEXT);
        }
    }
    Ok(WindowTarget {
        app: app.to_owned(),
        title: payload.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned),
        index: payload.index,
    })
}

fn position(payload: &WindowsPayload) -> Result<(i32, i32), &'static str> {
    match (payload.x, payload.y) {
        (Some(x), Some(y)) if (-20_000..=20_000).contains(&x) && (-20_000..=20_000).contains(&y) => {
            Ok((x, y))
        }
        _ => Err(BAD_POSITION_TEXT),
    }
}

fn size(payload: &WindowsPayload) -> Result<(u32, u32), &'static str> {
    match (payload.width, payload.height) {
        (Some(width), Some(height)) if (100..=10_000).contains(&width) && (100..=10_000).contains(&height) => {
            Ok((width, height))
        }
        _ => Err(BAD_SIZE_TEXT),
    }
}

/// Elige una ventana por índice (1-based), título exacto, título contenido o foco.
pub fn resolve_target(
    app: &str,
    windows: &[WindowInfo],
    title: Option<&str>,
    index: Option<u8>,
) -> Result<usize, WindowFailure> {
    if let Some(index) = index {
        let position = usize::from(index).saturating_sub(1);
        return if position < windows.len() {
            Ok(position)
        } else {
            Err(WindowFailure::Stable(format!("no encuentro una ventana de «{app}»")))
        };
    }
    if let Some(title) = title {
        let wanted = title.to_lowercase();
        let exact: Vec<usize> = windows
            .iter()
            .enumerate()
            .filter(|(_, w)| w.title.to_lowercase() == wanted)
            .map(|(i, _)| i)
            .collect();
        let matches = if exact.is_empty() {
            windows
                .iter()
                .enumerate()
                .filter(|(_, w)| w.title.to_lowercase().contains(&wanted))
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        } else {
            exact
        };
        return match matches.as_slice() {
            [only] => Ok(*only),
            [] => Err(WindowFailure::Stable(format!("no encuentro una ventana de «{app}»"))),
            many => Err(WindowFailure::Stable(format!(
                "hay {} ventanas de «{app}» que coinciden; concreta el título o el índice",
                many.len()
            ))),
        };
    }
    windows
        .iter()
        .position(|w| w.focused)
        .or(if windows.is_empty() { None } else { Some(0) })
        .ok_or_else(|| WindowFailure::Stable(format!("no encuentro una ventana de «{app}»")))
}

fn failure_output(
    action: WindowAction,
    failure: WindowFailure,
) -> crate::Result<OutputPayload> {
    let text = match failure {
        WindowFailure::Stable(text) => text,
        WindowFailure::Detail(detail) => {
            let verb = match action {
                WindowAction::MoveWindow => "mover",
                WindowAction::ResizeWindow => "redimensionar",
                WindowAction::FocusWindow => "traer",
                WindowAction::ListWindows => "listar",
            };
            format!("no se pudo {verb} la ventana: {detail}")
        }
    };
    OutputPayload::text(text)
}

pub fn handle_windows(
    bridge: &dyn WindowBridge,
    payload: &WindowsPayload,
) -> crate::Result<OutputPayload> {
    match payload.action {
        WindowAction::ListWindows => {
            if !bridge.permission() {
                return OutputPayload::text(NO_PERMISSION_TEXT);
            }
            let filter = payload.app.as_deref().map(str::trim).filter(|a| !a.is_empty());
            match bridge.list(filter) {
                Ok(mut windows) => {
                    windows.truncate(MAX_WINDOWS);
                    OutputPayload::Windows(WindowsOutput {
                        windows: windows.iter().map(WindowInfo::record).collect(),
                    })
                }
                Err(failure) => failure_output(payload.action, failure),
            }
        }
        WindowAction::MoveWindow => {
            let target = match build_target(payload) {
                Ok(target) => target,
                Err(text) => return OutputPayload::text(text),
            };
            let (x, y) = match position(payload) {
                Ok(position) => position,
                Err(text) => return OutputPayload::text(text),
            };
            if !bridge.permission() {
                return OutputPayload::text(NO_PERMISSION_TEXT);
            }
            match bridge.move_to(&target, x, y) {
                Ok(info) => OutputPayload::text(format!(
                    "Ventana «{}» de «{}» movida a {},{}",
                    info.title, info.app, info.x, info.y
                )),
                Err(failure) => failure_output(payload.action, failure),
            }
        }
        WindowAction::ResizeWindow => {
            let target = match build_target(payload) {
                Ok(target) => target,
                Err(text) => return OutputPayload::text(text),
            };
            let (width, height) = match size(payload) {
                Ok(size) => size,
                Err(text) => return OutputPayload::text(text),
            };
            if !bridge.permission() {
                return OutputPayload::text(NO_PERMISSION_TEXT);
            }
            match bridge.resize(&target, width, height) {
                Ok(info) => OutputPayload::text(format!(
                    "Ventana «{}» de «{}» redimensionada a {}×{}",
                    info.title, info.app, info.width, info.height
                )),
                Err(failure) => failure_output(payload.action, failure),
            }
        }
        WindowAction::FocusWindow => {
            let target = match build_target(payload) {
                Ok(target) => target,
                Err(text) => return OutputPayload::text(text),
            };
            if !bridge.permission() {
                return OutputPayload::text(NO_PERMISSION_TEXT);
            }
            match bridge.focus(&target) {
                Ok(info) => OutputPayload::text(format!(
                    "Ventana «{}» de «{}» traída al frente",
                    info.title, info.app
                )),
                Err(failure) => failure_output(payload.action, failure),
            }
        }
    }
}
```

- [ ] **Step 3: Pruebas del módulo**

Añadir al final de `windows.rs` un `#[cfg(test)]` con un `FakeBridge` que registre llamadas y devuelva `WindowInfo`; cubrir:
- `resolve_target` por índice 1-based, título exacto, contiene, ambiguo (2), sin coincidencia y foco por defecto.
- `build_target` sin app → `falta app`; índice 0/100 → texto.
- `position`/`size` fuera de rango y válidos.
- `handle_windows`: list sin permiso → texto y sin llamar al puente; list con permiso → `OutputPayload::Windows` truncado a 30; move/resize/focus → textos exactos y orden validación→permiso→puente; `WindowFailure::Stable` pasa tal cual y `Detail` se envuelve.
- `list` con app vacía se trata como sin filtro.

En `lib.rs`: `pub mod windows;` y `pub use windows::handle_windows;`.

- [ ] **Step 4: Compilar y probar**

Run: `cargo test -p atlas-mac` (RED inicial por tipos ausentes, luego GREEN).

- [ ] **Step 5: Commit**

```bash
git add crates/atlas-mac/src/wire.rs crates/atlas-mac/src/windows.rs crates/atlas-mac/src/lib.rs crates/atlas-mac/tests/wire.rs
git commit -m "feat(hefesto): módulo puro de ventanas y wire v7"
```

---

### Task 3: Despacho en el ejecutor y runtime

**Files:**
- Modify: `crates/atlas-mac/src/executor.rs`, `crates/atlas-mac/src/runtime.rs`, `crates/atlas-mac/tests/executor.rs`

**Interfaces:**
- Consumes: `windows::handle_windows`, `WindowBridge`.
- Produces: `Executor::with_window_bridge`, `Runtime::with_window_bridge`, brazo `(ToolId::Windows, ToolArguments::Windows)` → `Hefesto no puede gestionar ventanas` sin puente.

- [ ] **Step 1: Tests primero**

En `tests/executor.rs`: `FakeWindows` que devuelve dos `WindowInfo`; helpers `windows_executor()` y `windows_invocation(payload)`. Probar:
- `list_windows` → `OutputPayload::Windows` con 2 ventanas (buscar `windows.len()`).
- `move_window` → «Ventana «Documento» de «TextEdit» movida a 10,20».
- sin puente → `Hefesto no puede gestionar ventanas`.

- [ ] **Step 2: Implementar**

En `executor.rs`: campo `window_bridge: Option<Arc<dyn crate::windows::WindowBridge>>`, `new` a `None`, `with_window_bridge`, y brazo antes del `_`:

```rust
            (ToolId::Windows, ToolArguments::Windows(payload)) => match &self.window_bridge {
                Some(bridge) => crate::windows::handle_windows(bridge.as_ref(), payload)?,
                None => OutputPayload::text("Hefesto no puede gestionar ventanas")?,
            },
```

En `runtime.rs`: campo, `with_window_bridge` y pass-through en `spawn` como apps/teclado.

- [ ] **Step 3: Suites y commit**

Run: `cargo test -p atlas-mac`
```bash
git add crates/atlas-mac/src/executor.rs crates/atlas-mac/src/runtime.rs crates/atlas-mac/tests/executor.rs
git commit -m "feat(hefesto): despacho de hefesto.windows"
```

---

### Task 4: Puente real con Accessibility

**Files:**
- Create: `src-tauri/src/hefesto_windows.rs`
- Modify: `src-tauri/src/lib.rs` (módulo + inyección)
- Modify: `src-tauri/Cargo.toml` (features `NSScreen` y `NSRunningApplication` de objc2-app-kit; sin crates nuevas)

**Interfaces:**
- Consumes: `windows::{WindowBridge, WindowTarget, WindowInfo, WindowFailure, resolve_target}`.
- Produces: `AxWindows::new(AppHandle)` implementando `WindowBridge`.

- [ ] **Step 1: FFI y helpers**

```rust
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::string::CFString;
use std::ffi::c_void;
use std::sync::mpsc;
use std::time::Duration;

use atlas_mac::windows::{
    resolve_target, WindowBridge, WindowFailure, WindowInfo, WindowTarget,
};
use objc2_app_kit::NSWorkspace;
use tauri::AppHandle;

type AXUIElementRef = *const c_void;
type CFTypeRef = *const c_void;
const AX_ERROR_SUCCESS: i32 = 0;
const AX_VALUE_CGPOINT: u32 = 1;
const AX_VALUE_CGSIZE: u32 = 2;
const MAIN_ROUNDTRIP: Duration = Duration::from_secs(5);
const MESSAGING_TIMEOUT: f32 = 2.0;

#[repr(C)]
struct CGPoint { x: f64, y: f64 }
#[repr(C)]
struct CGSize { width: f64, height: f64 }

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(element: AXUIElementRef, attribute: CFTypeRef, value: *mut CFTypeRef) -> i32;
    fn AXUIElementSetAttributeValue(element: AXUIElementRef, attribute: CFTypeRef, value: CFTypeRef) -> i32;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFTypeRef) -> i32;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
}
```

Helpers (todos con justificación `unsafe` puntual):
- `ax_string(element, name) -> Option<String>`: `CFString::new(name)`, `CopyAttributeValue`, `CFType::wrap_under_create_rule`, `downcast::<CFString>()`.
- `ax_bool(element, name) -> Option<bool>`: `CFBoolean` (`bool_value()`).
- `ax_point(element, name)` / `ax_size(element, name)`: copy attr → `AXValueGetValue`.
- `set_point` / `set_size`: `AXValueCreate` + `CFType::wrap_under_create_rule` + `AXUIElementSetAttributeValue("AXPosition"/"AXSize")`.
- `ax_windows(element) -> Vec<AXUIElementRef>` envueltos en `CFType` con `wrap_under_get_rule` y guardados en un `Vec<CFType>` (los AXUIElement creados por `AXUIElementCreateApplication` se envuelven con create rule).
- `run_on_main` igual que teclado (canal + `run_on_main_thread` + timeout).
- `running_pid(app) -> Result<(i32, String), WindowFailure>`: `NSWorkspace::sharedWorkspace().runningApplications()`, política `Regular`, comparar `localizedName`/`bundleIdentifier` sin distinguir mayúsculas; si no está → `Stable("«X» no está abierta")`.
- `list_windows(pid, app_name) -> Vec<WindowInfo>`: por cada ventana, `role == "AXWindow"` y `subrole == "AXStandardWindow"`, leer título/posición/tamaño/foco/minimizado; título vacío → `"Sin título"`.
- Acotado: unir `NSScreen::screens()` (`frame`, origen inferior-izquierdo → convertir a AX arriba-izquierda con `main_screen.frame().size.height`), clamp de `x` a `[min_x, max_x - 200]` y `y` a `[min_y, max_y - 40]`.

- [ ] **Step 2: Operaciones**

- `permission()`: `AXIsProcessTrusted()`.
- `list(app)`: en el hilo principal; si `app` → resolver pid o devolver `Stable`; si no, recorrer `runningApplications` regulares y concatenar; cortar a 30 en el módulo.
- `move_to` / `resize`: resolver pid y ventanas, `resolve_target`, fijar atributo, releer `WindowInfo` y devolverlo; errores AX → `Detail(format!("AX {}", code))`.
- `focus`: `resolve_target`, set `AXMinimized=false`, `AXRaise`, set `AXFocused=true`, activar la app con `NSRunningApplication::activateWithOptions` (`ActivateAllWindows`); devolver `WindowInfo`.
- En `lib.rs`: `mod hefesto_windows;` y `.with_window_bridge(Arc::new(hefesto_windows::AxWindows::new(handle.clone())))` junto a los otros puentes.

- [ ] **Step 3: Compilar y commit**

Run: `cargo check --workspace && cargo fmt --check && cargo clippy -p atlas-next --all-targets -- -D warnings`
```bash
git add src-tauri/src/hefesto_windows.rs src-tauri/src/lib.rs src-tauri/Cargo.toml Cargo.lock
git commit -m "feat(hefesto): puente real de ventanas con Accessibility"
```

---

### Task 5: Journal v7 con `hefesto.windows`

**Files:**
- Modify: `server/hefesto.py`, `server/test_hefesto.py`

**Interfaces:**
- Consumes: protocolo v7.
- Produces: `submit` acepta `hefesto.windows` con acciones `list_windows|move_window|resize_window|focus_window`, claves cerradas por acción y `side_effects`; sin `_contents`.

- [ ] **Step 1: Tests**

`WindowsJournalTests(JournalTests)` con:
- `list_windows` válido → `next`/`result` con `{windows:[...]}`.
- `move_window` válido → arguments exactos.
- `action` inválida → `bad_action`.
- `move_window` sin app → `bad_argument`; `x` fuera de rango → `bad_argument`; `resize_window` con 99 → `bad_argument`; `index` 0 → `bad_argument`; clave extra `content` → `bad_argument`.
- reinicio con `move_window` pendiente → `unknown` y `next()` vacío.
- `list_windows` pendiente tras reinicio → `unknown` (conservador, documentado).

- [ ] **Step 2: Implementar**

Constantes junto a las de media:

```python
_WINDOW_ACTIONS = ("list_windows", "move_window", "resize_window", "focus_window")
_WINDOW_KEYS = {
    "list_windows": {"action", "app"},
    "move_window": {"action", "app", "title", "index", "x", "y"},
    "resize_window": {"action", "app", "title", "index", "width", "height"},
    "focus_window": {"action", "app", "title", "index"},
}
```

Rama en `_validate`:

```python
        elif body["toolId"] == "hefesto.windows":
            action = args.get("action")
            if action not in _WINDOW_ACTIONS:
                raise _Rejected("bad_action", 400)
            if not set(args) <= _WINDOW_KEYS[action]:
                raise _Rejected("bad_argument", 400)
            if action != "list_windows":
                app = args.get("app")
                if type(app) is not str or not app.strip() or len(app) > 256:
                    raise _Rejected("bad_argument", 400)
            if "title" in args and (type(args["title"]) is not str or len(args["title"]) > 256):
                raise _Rejected("bad_argument", 400)
            if "index" in args and (type(args["index"]) is not int or not 1 <= args["index"] <= 99):
                raise _Rejected("bad_argument", 400)
            for key in ("x", "y"):
                if key in args and (type(args[key]) is not int or not -20000 <= args[key] <= 20000):
                    raise _Rejected("bad_argument", 400)
            for key in ("width", "height"):
                if key in args and (type(args[key]) is not int or not 100 <= args[key] <= 10000):
                    raise _Rejected("bad_argument", 400)
```

Añadir `"hefesto.windows"` a la tupla de tools y a `side_effects` (sin `_contents`). La salida `{windows:[...]}` debe validarse: añadir una función `_windows_output(value)` análoga a `_apps_output` (lista ≤30 de dicts con claves `app,title,x,y,width,height,focused,minimized` y tipos correctos) y aceptarla en el `output` de `_validate` y en el `result`.

- [ ] **Step 3: Suites y commit**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
```bash
git add server/hefesto.py server/test_hefesto.py
git commit -m "feat(hefesto): journal acepta hefesto.windows"
```

---

### Task 6: Relay `hefesto_windows` y órbita

**Files:**
- Modify: `server/atlas_relay.py`, `server/test_hefesto_delegation.py`
- Modify: `src/state/orbitPlan.ts`, `src/state/orbitPlan.test.ts`

**Interfaces:**
- Consumes: journal v7.
- Produces: herramienta `hefesto_windows` con acciones del modelo `list|move|resize|focus` traducidas a las del wire; `list` sin tarjeta; efectos con una tarjeta; sin burbuja; órbita «Hefesto · Ventanas».

- [ ] **Step 1: Tests del relay**

`WindowsFlowTests`:
- catálogo: enum `["list","move","resize","focus"]`, `requiresConfirmation` y `planConfirmation` true.
- `list` → sin tarjeta, submit `hefesto.windows` con `{"action":"list_windows"}` (o con `app` normalizada), y el resultado estructurado se formatea a texto (`«TextEdit» · «Documento» — 10,20 800×600 [foco]`).
- `move` → tarjeta «Mover la ventana de TextEdit a 10,20» (riesgo `write`), submit `{"action":"move_window","app":"TextEdit","x":10,"y":20}`, texto devuelto.
- `resize`/`focus` → tarjeta y args correctos.
- `action` inválida / falta app / x fuera de rango / width 99 → textos estables, cero tarjetas, cero submits.
- éxito de efecto → `_hefesto_note` no llamado.
- `_handle_tool_call`: `list` abre 0 tarjetas; `move` abre 1.

- [ ] **Step 2: Implementar catálogo y validación**

`HEFESTO_WINDOWS_TOOL` con `requiresConfirmation: True`, `planConfirmation: True`, enum del modelo, propiedades `app`, `title`, `index` (1–99), `x`/`y` (-20000–20000), `width`/`height` (100–10000). Añadir a `HEFESTO_TOOLS`.

`_windows(job, args)`:
- validar acción del modelo y campos con los textos estables;
- mapear acción → wire; construir `arguments` con solo las claves de esa acción;
- `list`: sin tarjeta, `_hefesto_submit` + `_hefesto_wait` + formatear `{windows}` a líneas (máx. 30), devolver texto;
- efectos: `job["deadline"] += CONFIRM_TIMEOUT_S + 60`, `_confirm_summary`, una tarjeta, luego submit/wait y devolver `text`;
- sin `_hefesto_note`.

`execute_tool`: `hefesto_windows` → `_windows`.

`_confirm_summary`: resúmenes «Listar ventanas de X» (no se usa, sin tarjeta), «Mover la ventana de X a x,y», «Redimensionar la ventana de X a W×H», «Traer al frente la ventana de X», riesgo `write`.

- [ ] **Step 3: Órbita**

`HEFESTO_PHASES.hefesto_windows = 'Hefesto · Ventanas'` y su prueba `toEqual`.

- [ ] **Step 4: Suites y commit**

Run: Python delegation + full; `npx vitest run src/state/orbitPlan.test.ts`.
```bash
git add server/atlas_relay.py server/test_hefesto_delegation.py src/state/orbitPlan.ts src/state/orbitPlan.test.ts
git commit -m "feat(hefesto): herramienta hefesto_windows y órbita"
```

---

### Task 7: Verificación, bundle, despliegue, en vivo y registro

**Files:**
- Modify: `docs/hefesto-plan.md` (§5, §6, §8), `docs/contracts.md`, `docs/implemented.md`, `docs/implementation-plan.md`

- [ ] **Step 1: Suites completas** (`cargo test --workspace`, clippy, fmt, Python, frontend, build, lint).
- [ ] **Step 2: Bundle firmado Apple Development + `codesign --verify --deep --strict` + `ditto` a `/Applications/Atlas.app`.**
- [ ] **Step 3: Desplegar relay con `./scripts/deploy-relay.sh` (dispositivo primero) y `/v1/health`.**
- [ ] **Step 4: Matriz en vivo (pantalla desbloqueada):** listar ventanas de TextEdit; mover a 100,150 y verificar; redimensionar a 900×700; abrir Safari y traer TextEdit al frente; ambigüedad con dos ventanas del mismo título; matriz de validación por HTTP; comprobar cero burbujas en efectos y cero tarjetas en `list`.
- [ ] **Step 5: Registro** en §8, contracts v7, implemented.md y implementation-plan.md; commit `docs(hefesto): registro de ventanas (puerta 7, corte 3)`.

## Self-Review

- Cobertura: v7 (T1), wire+puro (T2), despacho (T3), puente AX (T4), journal (T5), relay+órbita (T6), verificación/registro (T7).
- Sin placeholders; los textos estables aparecen en código y pruebas.
- Consistencia: `WindowFailure` se define en T2 y lo usan T3/T4; las acciones del wire se definen una vez y el relay las mapea; `WindowsOutput` se valida en journal y se formatea en relay.
- Riesgo principal: el FFI de AX (`CFArray<*const c_void>` + `wrap_under_get_rule` + `AXValueCreate/GetValue`); T4 lo compila y T7 lo prueba en vivo.
