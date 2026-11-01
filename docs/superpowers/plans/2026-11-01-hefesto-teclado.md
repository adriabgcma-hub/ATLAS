# Teclado sintético de Hefesto (puerta 7, corte 2) — Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Que Hefesto escriba texto y pulse un atajo en la app que está en primer plano, siempre con tarjeta de confirmación, sin saltos de línea, respetando la entrada segura de macOS y sin persistir ni registrar el texto.

**Architecture:** El dispositivo (`crates/atlas-mac`) enruta un `toolId` nuevo `hefesto.keyboard` a un módulo `keyboard.rs` de lógica pura (validación del texto y gramática del atajo, textos estables) detrás de un trait inyectado `KeyboardBridge`; `src-tauri` implementa el puente con CGEvent en el hilo principal, comprueba el permiso de publicación de eventos (TCC) y la entrada segura, y pide el permiso una vez por arranque. El relay (Python) expone **una** herramienta `hefesto_keyboard` (`requiresConfirmation: True`), valida antes de delegar, redacta el texto de los logs y **no** publica burbuja. El journal trata `hefesto.keyboard` como efecto con contenido: el texto vive solo en memoria y una invocación en vuelo queda `unknown` tras reiniciar.

**Tech Stack:** Rust (Tauri v2, `objc2`/`objc2-app-kit`, `core-graphics` para CGEvent), Python 3 (relay + journal `sqlite3`), TypeScript/React + vitest.

**Spec:** [docs/superpowers/specs/2026-11-01-hefesto-teclado-design.md](file:///Users/adria/Developer/project-1/atlas-next/docs/superpowers/specs/2026-11-01-hefesto-teclado-design.md)

## Global Constraints

- `protocolVersion` sube de **4 a 5** en los dos extremos (wire Rust y Python) y en todos los fixtures/tests; ambos lados deben rechazar versiones distintas.
- `toolId` del dispositivo: **`hefesto.keyboard`**. Nombre de herramienta del relay: **`hefesto_keyboard`**.
- **Una sola** herramienta con `action ∈ {type, combo}`.
- `requiresConfirmation: True` **siempre**; nunca se ejecuta sin `allow_once` ni existe ruta trivial.
- Destino: **solo la app en primer plano**; no se activan apps, no se eligen ventanas ni campos.
- `type`: `text` no vacío, **≤4096 bytes UTF-8**, sin `\n`, `\r`, `\t` ni caracteres de control (`Cc`).
- `combo`: `mods+tecla`, **1–3** modificadores de `cmd|shift|alt|ctrl` sin repetir, y una tecla de `a-z`, `0-9`, `space`, `return`, `tab`, `esc`, `left`, `right`, `up`, `down`, `delete`; `combo` ≤64 bytes. **Al menos un modificador.**
- Entrada segura activa → **no se postea nada** y error estable. Sin permiso TCC → un diálogo del sistema por arranque y error estable, sin efectos.
- El texto tecleado **nunca** se persiste, **nunca** se escribe en el log del relay y **nunca** se replica en burbuja.
- CGEvent es API pública; nada de APIs privadas. Todo el trabajo AppKit/CG va en el hilo principal con `run_on_main_thread` y timeout.
- Journal: `hefesto.keyboard` es efecto con contenido (solo `textBytes`/`textSha256` persistidos, texto en memoria); invocación en vuelo tras reinicio → `unknown`, sin reejecutar.
- Regla del Mac: sin procesos nuevos que queden vivos ni RAM añadida al cerrar la prueba.
- Idioma del registro: commits en español con alcance `feat(hefesto)` / `fix(hefesto)` / `docs(hefesto)`.

---

### Task 1: Bump de protocolo v4 → v5

**Files:**
- Modify: `crates/atlas-mac/src/wire.rs:51-68` (`ProtocolVersion::try_from` y `From<ProtocolVersion> for u64`)
- Modify: literales `protocolVersion == 4` / `"protocolVersion":4` en `crates/atlas-mac/src/client.rs`, `crates/atlas-mac/src/runtime.rs` (tests), `crates/atlas-mac/src/executor.rs` (tests), `crates/atlas-mac/tests/*.rs`, `server/hefesto.py`, `server/atlas_relay.py`, `server/test_hefesto.py`, `server/test_hefesto_delegation.py`

**Interfaces:**
- Consumes: nada.
- Produces: `ProtocolVersion` que serializa a `5` y acepta solo `5`. Todas las tareas siguientes asumen 5.

- [ ] **Step 1: Localizar todos los literales de versión**

Run: `cd /Users/adria/Developer/project-1/atlas-next && rg -n 'protocolVersion' crates server | rg '4'`
Deja a la vista cada `4` de versión (no confundir con timeouts ni tamaños).

- [ ] **Step 2: Escribir el test fallido de versión**

En `crates/atlas-mac/src/wire.rs`, en `#[cfg(test)] mod tests`, cambia el test existente:

```rust
#[test]
fn protocol_version_is_five() {
    assert_eq!(u64::from(ProtocolVersion), 5);
    assert!(ProtocolVersion::try_from(5u64).is_ok());
    assert!(ProtocolVersion::try_from(4u64).is_err());
}
```

- [ ] **Step 3: Ejecutar para ver que falla**

Run: `cargo test -p atlas-mac protocol_version_is_five`
Expected: FAIL (`5` no es aceptado todavía).

- [ ] **Step 4: Cambiar el wire y los literales**

En `wire.rs`: `if value == 5` y `5` en `From`. Después sustituye cada literal de versión en código y tests (`== 4` → `== 5`, `"protocolVersion":4` → `"protocolVersion":5`, `protocolVersion=4` → `5`), incluidos `server/hefesto.py` (docstring y validación), `server/atlas_relay.py` (submits/outcome/errors) y los tests: el `FakeJournal` de `test_hefesto_delegation.py` y el `ok()` de `JournalTests` en `test_hefesto.py` hardcodean `protocolVersion: 4`. No toques otros `4` (p. ej. `MAX_TURNS`).

- [ ] **Step 5: Ejecutar y pasar**

Run: `cargo test -p atlas-mac && cargo test --workspace`
Run: `cd server && python3 -m unittest discover -p "test_*.py"`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/atlas-mac/src/wire.rs crates/atlas-mac/src/client.rs crates/atlas-mac/src/runtime.rs crates/atlas-mac/src/executor.rs crates/atlas-mac/tests server/hefesto.py server/atlas_relay.py server/test_hefesto.py server/test_hefesto_delegation.py
git commit -m "feat(hefesto): protocolo v5 de preparación para teclado"
```

---

### Task 2: Módulo `keyboard.rs` puro + tipos wire + trait `KeyboardBridge`

**Files:**
- Create: `crates/atlas-mac/src/keyboard.rs`
- Modify: `crates/atlas-mac/src/wire.rs` (tipos `KeyboardAction`, `KeyboardPayload`, `ToolId::Keyboard`, `ToolArguments::Keyboard`)
- Modify: `crates/atlas-mac/src/lib.rs` (`pub mod keyboard;` y re-export del trait y constantes)
- Test: tests dentro de `keyboard.rs` y en `crates/atlas-mac/tests/wire.rs`

**Interfaces:**
- Consumes: `wire::{KeyboardAction, KeyboardPayload, OutputPayload}`.
- Produces: `KeyboardBridge` con `can_post(&self, prompt: bool) -> bool`, `secure_input(&self) -> bool`, `frontmost_app(&self) -> Option<String>`, `type_text(&self, text: &str) -> Result<(), String>`, `press(&self, modifiers: u8, keycode: u16) -> Result<(), String>`; constantes `MOD_CMD=1`, `MOD_SHIFT=2`, `MOD_ALT=4`, `MOD_CTRL=8`; `handle_keyboard(&dyn KeyboardBridge, &KeyboardPayload) -> crate::Result<OutputPayload>`; `parse_combo(&str) -> Result<Combo, &'static str>` con `pub struct Combo { pub modifiers: u8, pub keycode: u16 }`.

- [ ] **Step 1: Escribir los tests fallidos (gramática, texto y textos estables)**

En `keyboard.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{KeyboardAction, KeyboardPayload};

    fn payload(action: KeyboardAction, text: Option<&str>, combo: Option<&str>) -> KeyboardPayload {
        KeyboardPayload { action, text: text.map(str::to_owned), combo: combo.map(str::to_owned) }
    }

    #[test]
    fn atajo_valido_con_modificadores() {
        let combo = parse_combo("cmd+shift+s").unwrap();
        assert_eq!(combo.modifiers, MOD_CMD | MOD_SHIFT);
        assert_eq!(combo.keycode, 1);
    }

    #[test]
    fn atajo_sin_modificador_se_rechaza() {
        assert!(parse_combo("return").is_err());
        assert!(parse_combo("s").is_err());
    }

    #[test]
    fn atajo_con_modificador_repetido_o_desconocido_se_rechaza() {
        assert!(parse_combo("cmd+cmd+s").is_err());
        assert!(parse_combo("meta+s").is_err());
        assert!(parse_combo("cmd+ñ").is_err());
    }

    #[test]
    fn atajo_con_teclas_especiales() {
        assert_eq!(parse_combo("cmd+space").unwrap().keycode, 49);
        assert_eq!(parse_combo("ctrl+left").unwrap().keycode, 123);
    }

    #[test]
    fn texto_con_saltos_o_controles_se_rechaza() {
        assert!(validate_text("hola").is_ok());
        assert!(validate_text("hola\nmundo").is_err());
        assert!(validate_text("hola\tmundo").is_err());
        assert!(validate_text("").is_err());
        assert!(validate_text(&"a".repeat(4097)).is_err());
    }

    #[test]
    fn type_exitoso_resume_caracteres() {
        let bridge = FakeKeyboard::ok(Some("TextEdit"));
        let out = handle_keyboard(&bridge, &payload(KeyboardAction::Type, Some("hola"), None)).unwrap();
        assert_eq!(out.text_value(), Some("Escrito 4 caracteres en «TextEdit»"));
    }

    #[test]
    fn combo_exitoso_resume_atajo_y_app() {
        let bridge = FakeKeyboard::ok(Some("TextEdit"));
        let out = handle_keyboard(&bridge, &payload(KeyboardAction::Combo, None, Some("cmd+s"))).unwrap();
        assert_eq!(out.text_value(), Some("Pulsado cmd+s en «TextEdit»"));
    }

    #[test]
    fn entrada_segura_no_teclea() {
        let bridge = FakeKeyboard { secure: true, ..FakeKeyboard::ok(Some("Safari")) };
        let out = handle_keyboard(&bridge, &payload(KeyboardAction::Type, Some("hola"), None)).unwrap();
        assert_eq!(out.text_value(), Some(SECURE_INPUT_TEXT));
        assert!(!bridge.called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn sin_permiso_no_teclea_y_pide_una_vez() {
        let bridge = FakeKeyboard { granted: false, ..FakeKeyboard::ok(Some("TextEdit")) };
        let out = handle_keyboard(&bridge, &payload(KeyboardAction::Combo, None, Some("cmd+s"))).unwrap();
        assert_eq!(out.text_value(), Some(NO_PERMISSION_TEXT));
        assert!(bridge.prompted.load(std::sync::atomic::Ordering::SeqCst));
    }
}
```

`FakeKeyboard` (en el mismo `mod tests`) con `granted: bool`, `secure: bool`, `app: Option<String>`, `called: AtomicBool`, `prompted: AtomicBool` e implementación del trait que marca `called`.

- [ ] **Step 2: Ejecutar para ver que falla**

Run: `cargo test -p atlas-mac keyboard`
Expected: FAIL (módulo/tipos inexistentes).

- [ ] **Step 3: Añadir los tipos wire**

En `wire.rs`, tras `AppsOutput`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyboardAction {
    Type,
    Combo,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyboardPayload {
    pub action: KeyboardAction,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub combo: Option<String>,
}
```

Añade `Keyboard(KeyboardPayload)` **al final** de `ToolArguments` (el enum es `untagged`: `AppsPayload` debe seguir antes para que `action:"list"` no caiga aquí) y:

```rust
    #[serde(rename = "hefesto.keyboard")]
    Keyboard,
```

en `ToolId`.

- [ ] **Step 4: Implementar `keyboard.rs`**

```rust
use crate::wire::{KeyboardAction, KeyboardPayload, OutputPayload};

pub const MOD_CMD: u8 = 1;
pub const MOD_SHIFT: u8 = 2;
pub const MOD_ALT: u8 = 4;
pub const MOD_CTRL: u8 = 8;

pub const MAX_TEXT_BYTES: usize = 4096;
pub const MAX_COMBO_BYTES: usize = 64;

pub const SECURE_INPUT_TEXT: &str =
    "hay un campo con entrada segura en primer plano; no tecleo ahí";
pub const NO_PERMISSION_TEXT: &str =
    "falta el permiso de Accesibilidad; actívalo en Ajustes del Sistema › Privacidad y seguridad › Accesibilidad";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Combo {
    pub modifiers: u8,
    pub keycode: u16,
}

pub trait KeyboardBridge: Send + Sync {
    fn can_post(&self, prompt: bool) -> bool;
    fn secure_input(&self) -> bool;
    fn frontmost_app(&self) -> Option<String>;
    fn type_text(&self, text: &str) -> Result<(), String>;
    fn press(&self, modifiers: u8, keycode: u16) -> Result<(), String>;
}

pub fn validate_text(text: &str) -> Result<(), &'static str> {
    if text.is_empty() {
        return Err("vacío");
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err("largo");
    }
    if text.chars().any(char::is_control) {
        return Err("controles");
    }
    Ok(())
}

fn keycode_for(key: &str) -> Option<u16> {
    Some(match key {
        "a" => 0, "s" => 1, "d" => 2, "f" => 3, "h" => 4, "g" => 5,
        "z" => 6, "x" => 7, "c" => 8, "v" => 9, "b" => 11, "q" => 12,
        "w" => 13, "e" => 14, "r" => 15, "y" => 16, "t" => 17,
        "1" => 18, "2" => 19, "3" => 20, "4" => 21, "6" => 22, "5" => 23,
        "9" => 25, "7" => 26, "8" => 28, "0" => 29,
        "o" => 31, "u" => 32, "i" => 34, "p" => 35, "l" => 37, "j" => 38,
        "k" => 40, "n" => 45, "m" => 46,
        "return" => 36, "tab" => 48, "space" => 49, "delete" => 51, "esc" => 53,
        "left" => 123, "right" => 124, "down" => 125, "up" => 126,
        _ => return None,
    })
}

pub fn parse_combo(combo: &str) -> Result<Combo, &'static str> {
    if combo.is_empty() || combo.len() > MAX_COMBO_BYTES {
        return Err("atajo no válido");
    }
    let parts: Vec<&str> = combo.split('+').collect();
    let Some((key, mods)) = parts.split_last() else {
        return Err("atajo no válido");
    };
    if mods.is_empty() || mods.len() > 3 {
        return Err("atajo no válido");
    }
    let mut modifiers = 0u8;
    for name in mods {
        let bit = match *name {
            "cmd" => MOD_CMD,
            "shift" => MOD_SHIFT,
            "alt" => MOD_ALT,
            "ctrl" => MOD_CTRL,
            _ => return Err("atajo no válido"),
        };
        if modifiers & bit != 0 {
            return Err("atajo no válido");
        }
        modifiers |= bit;
    }
    let keycode = keycode_for(key).ok_or("atajo no válido")?;
    Ok(Combo { modifiers, keycode })
}

fn app_label(bridge: &dyn KeyboardBridge) -> String {
    match bridge.frontmost_app() {
        Some(name) if !name.trim().is_empty() => format!("«{name}»"),
        _ => "la app en primer plano".to_string(),
    }
}

pub fn handle_keyboard(
    bridge: &dyn KeyboardBridge,
    payload: &KeyboardPayload,
) -> crate::Result<OutputPayload> {
    match payload.action {
        KeyboardAction::Type => {
            let Some(text) = payload.text.as_deref() else {
                return OutputPayload::text("falta text para type");
            };
            if validate_text(text).is_err() {
                return OutputPayload::text("texto no válido (sin saltos de línea ni controles)");
            }
            if bridge.secure_input() {
                return OutputPayload::text(SECURE_INPUT_TEXT);
            }
            if !bridge.can_post(true) {
                return OutputPayload::text(NO_PERMISSION_TEXT);
            }
            match bridge.type_text(text) {
                Ok(()) => {
                    let count = text.chars().count();
                    let unit = if count == 1 { "carácter" } else { "caracteres" };
                    OutputPayload::text(format!("Escrito {count} {unit} en {}", app_label(bridge)))
                }
                Err(err) => OutputPayload::text(format!("no se pudo teclear: {err}")),
            }
        }
        KeyboardAction::Combo => {
            let Some(combo) = payload.combo.as_deref() else {
                return OutputPayload::text("falta combo para combo");
            };
            let Ok(plan) = parse_combo(combo) else {
                return OutputPayload::text("atajo no válido");
            };
            if bridge.secure_input() {
                return OutputPayload::text(SECURE_INPUT_TEXT);
            }
            if !bridge.can_post(true) {
                return OutputPayload::text(NO_PERMISSION_TEXT);
            }
            match bridge.press(plan.modifiers, plan.keycode) {
                Ok(()) => OutputPayload::text(format!(
                    "Pulsado {combo} en {}",
                    app_label(bridge)
                )),
                Err(err) => OutputPayload::text(format!("no se pudo pulsar: {err}")),
            }
        }
    }
}
```

- [ ] **Step 5: Exportar y ejecutar**

En `lib.rs`: `pub mod keyboard;` y `pub use keyboard::{handle_keyboard, KeyboardBridge};`.

En `crates/atlas-mac/tests/wire.rs` añade un test de fixture v5 con `{"action":"type","text":"hola"}` que deserializa a `ToolArguments::Keyboard` y otro con `{"action":"combo","combo":"cmd+s"}`.

Run: `cargo test -p atlas-mac && cargo clippy -p atlas-mac --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/atlas-mac/src/keyboard.rs crates/atlas-mac/src/wire.rs crates/atlas-mac/src/lib.rs crates/atlas-mac/tests/wire.rs
git commit -m "feat(hefesto): módulo teclado con gramática cerrada y trait KeyboardBridge"
```

---

### Task 3: Despacho `hefesto.keyboard` en el ejecutor e inyección en el runtime

**Files:**
- Modify: `crates/atlas-mac/src/executor.rs` (campo, constructor, builder, rama del `match`)
- Modify: `crates/atlas-mac/src/runtime.rs` (campo, constructor, builder, paso al ejecutor)
- Test: `crates/atlas-mac/tests/executor.rs`

**Interfaces:**
- Consumes: `keyboard::{KeyboardBridge, handle_keyboard}` (Task 2).
- Produces: `Executor::with_keyboard_bridge(Arc<dyn KeyboardBridge>)` y `Runtime::with_keyboard_bridge(Arc<dyn KeyboardBridge>)`.

- [ ] **Step 1: Escribir el test fallido**

En `crates/atlas-mac/tests/executor.rs`, junto a los de apps (usa los helpers
existentes `apps_executor`, `invocation()`, `id()` y `at()`):

```rust
struct FakeKeyboard;
impl atlas_mac::KeyboardBridge for FakeKeyboard {
    fn can_post(&self, _: bool) -> bool { true }
    fn secure_input(&self) -> bool { false }
    fn frontmost_app(&self) -> Option<String> { Some("TextEdit".into()) }
    fn type_text(&self, _: &str) -> std::result::Result<(), String> { Ok(()) }
    fn press(&self, _: u8, _: u16) -> std::result::Result<(), String> { Ok(()) }
}

fn keyboard_executor() -> Executor {
    apps_executor(None).with_keyboard_bridge(Arc::new(FakeKeyboard))
}

fn keyboard_invocation(action: KeyboardAction, text: Option<&str>, combo: Option<&str>, inv_id: &str) -> Invocation {
    let mut inv = invocation();
    inv.invocation_id = id(inv_id);
    inv.tool_id = ToolId::Keyboard;
    inv.arguments = ToolArguments::Keyboard(KeyboardPayload {
        action,
        text: text.map(str::to_owned),
        combo: combo.map(str::to_owned),
    });
    inv
}

#[test]
fn keyboard_type_se_enruta_por_el_puente_inyectado() {
    let mut executor = keyboard_executor();
    let out = executor
        .execute(&keyboard_invocation(KeyboardAction::Type, Some("hola"), None, "k-type"), at(3))
        .unwrap();
    assert_eq!(out.output.text_value(), Some("Escrito 4 caracteres en «TextEdit»"));
}

#[test]
fn keyboard_combo_se_enruta_por_el_puente_inyectado() {
    let mut executor = keyboard_executor();
    let out = executor
        .execute(&keyboard_invocation(KeyboardAction::Combo, None, Some("cmd+s"), "k-combo"), at(3))
        .unwrap();
    assert_eq!(out.output.text_value(), Some("Pulsado cmd+s en «TextEdit»"));
}

#[test]
fn keyboard_sin_puente_devuelve_texto_no_disponible() {
    let mut executor = apps_executor(None);
    let out = executor
        .execute(&keyboard_invocation(KeyboardAction::Type, Some("hola"), None, "k-none"), at(3))
        .unwrap();
    assert_eq!(out.output.text_value(), Some("Hefesto no puede teclear"));
}
```

- [ ] **Step 2: Ejecutar para ver que falla**

Run: `cargo test -p atlas-mac keyboard_se_enruta`
Expected: FAIL (no existe `with_keyboard_bridge`).

- [ ] **Step 3: Implementar**

En `executor.rs`, replica el patrón de `app_bridge`:

```rust
    keyboard_bridge: Option<std::sync::Arc<dyn crate::keyboard::KeyboardBridge>>,
```
con `None` en el constructor:

```rust
    pub fn with_keyboard_bridge(
        mut self,
        bridge: std::sync::Arc<dyn crate::keyboard::KeyboardBridge>,
    ) -> Self {
        self.keyboard_bridge = Some(bridge);
        self
    }
```

y en el `match`, junto a la rama de Apps:

```rust
            (ToolId::Keyboard, ToolArguments::Keyboard(payload)) => match &self.keyboard_bridge {
                Some(bridge) => crate::keyboard::handle_keyboard(bridge.as_ref(), payload)?,
                None => OutputPayload::text("Hefesto no puede teclear")?,
            },
```

En `runtime.rs`, replica `app_bridge` con `keyboard_bridge` y págalo al ejecutor donde ya se pasa `app_bridge`.

- [ ] **Step 4: Ejecutar y pasar**

Run: `cargo test -p atlas-mac && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/atlas-mac/src/executor.rs crates/atlas-mac/src/runtime.rs crates/atlas-mac/tests/executor.rs
git commit -m "feat(hefesto): despacho hefesto.keyboard con puente inyectado"
```

---

### Task 4: Puente real con CGEvent en `src-tauri`

**Files:**
- Modify: `src-tauri/Cargo.toml` (añadir `core-graphics = "0.24"`)
- Create: `src-tauri/src/hefesto_keyboard.rs`
- Modify: `src-tauri/src/lib.rs` (`mod hefesto_keyboard;`, `use`, `.with_keyboard_bridge(...)`)

**Interfaces:**
- Consumes: `atlas_mac::keyboard::{KeyboardBridge, MOD_*}` (Task 2), `tauri::AppHandle`.
- Produces: `pub struct CgKeyboard { handle: AppHandle, prompted: AtomicBool }` que implementa `KeyboardBridge`.

- [ ] **Step 1: Añadir la dependencia**

En `src-tauri/Cargo.toml`:

```toml
core-graphics = "0.24"
```

- [ ] **Step 2: Implementar el puente**

`hefesto_keyboard.rs`. Puntos obligatorios (el compilador de `cargo check` es el
oráculo para los nombres exactos de la API de `core-graphics`; **no cambies el
contrato del trait ni los textos**):

- `unsafe` acotado y documentado para las dos funciones C que `core-graphics` no expone:

```rust
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
}

#[link(name = "HIToolbox", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}
```

- `can_post(prompt)`: si `CGPreflightPostEventAccess()` es `true` → `true`. Si no y
  `prompt` y `!self.prompted.swap(true, Ordering::SeqCst)` → llama a
  `CGRequestPostEventAccess()` (abre el diálogo una vez por arranque) y devuelve el
  resultado de una nueva preflight.
- `secure_input()`: `IsSecureEventInputEnabled()`.
- `frontmost_app()`: `NSWorkspace::sharedWorkspace().frontmostApplication()` y su
  `localizedName()`, **en el hilo principal**.
- `type_text(text)`: en el hilo principal, para cada trozo de ≤20 unidades UTF-16 crea
  un evento de teclado con `CGEvent::new_keyboard_event(source, 0, true)`,
  `event.set_string(chunk)`, `event.post(CGEventTapLocation::HID)` y otro evento
  `key_down=false` con el mismo texto; usa `CGEventSource::new(HIDSystemState)`.
- `press(modifiers, keycode)`: en el hilo principal, traduce los bits `MOD_CMD=1`,
  `MOD_SHIFT=2`, `MOD_ALT=4`, `MOD_CTRL=8` a `CGEventFlags::CGEventFlagCommand` /
  `CGEventFlagShift` / `CGEventFlagAlternate` / `CGEventFlagControl`, y postea
  `key_down=true` y `false` con `set_flags`.
- **Todo** método que toque AppKit/CG se reenvía con
  `self.handle.run_on_main_thread(...)` + `mpsc` + `recv_timeout(Duration::from_secs(5))`,
  igual que `WorkspaceApps`. Nunca se duerme en el hilo principal.
- Los errores del canal devuelven `String` con textos cortos
  (`"no se pudo reenviar al hilo principal"`, `"timeout del hilo principal"`).

- [ ] **Step 3: Cablear en el arranque**

En `lib.rs`, junto al puente de apps:

```rust
let runtime = Runtime::new(client, device_id, roots)
    .with_app_bridge(Arc::new(WorkspaceApps::new(app.clone())))
    .with_keyboard_bridge(Arc::new(CgKeyboard::new(app.clone())));
```

- [ ] **Step 4: Compilar**

Run: `set -a; . ./.env 2>/dev/null; set +a; cargo check --workspace`
Expected: PASS.

Run: `cargo clippy -p atlas-next --all-targets -- -D warnings`
Expected: sin avisos; el `unsafe` queda justificado con un comentario de una línea en
cada bloque `extern`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/hefesto_keyboard.rs src-tauri/src/lib.rs
git commit -m "feat(hefesto): puente CGEvent con permiso y entrada segura en el hilo principal"
```

---

### Task 5: Journal v5 con `hefesto.keyboard` sin texto persistido

**Files:**
- Modify: `server/hefesto.py` (toolIds admitidos, validación, redacción, `_contents`, `next`, side-effects, validación de output)
- Modify: `server/test_hefesto.py`

**Interfaces:**
- Consumes: protocolo v5 (Task 1).
- Produces: `submit` acepta `hefesto.keyboard` con `{action, text?|combo?}`; nunca persiste texto; `next` lo entrega desde memoria; sin entrega tras reinicio → `unknown`.

- [ ] **Step 1: Escribir los tests fallidos**

En `server/test_hefesto.py`, junto a `APPS_OPEN` y `WriteJournalTests`:

```python
KEYBOARD_TYPE = {"submissionId": "submission-kbd", "requestId": "request-1",
                 "sessionId": "session-1", "taskId": "task-1",
                 "toolId": "hefesto.keyboard",
                 "arguments": {"action": "type", "text": "secreto"}}


class KeyboardJournalTests(JournalTests):
    def test_el_texto_no_se_guarda_en_sqlite(self):
        self.ready()
        self.ok("submit", peer=CONTROLLER, **KEYBOARD_TYPE)
        invocation = self.next()
        self.assertEqual(invocation["arguments"]["text"], "secreto")
        with sqlite3.connect(self.path) as db:
            submission, stored_text = db.execute(
                "SELECT submission, text FROM invocations").fetchone()
        self.assertNotIn("secreto", submission)
        self.assertEqual(stored_text, "")
        self.assertIn("textSha256", submission)

    def test_reinicio_sin_entregar_deja_unknown_y_no_reenvia(self):
        self.ready()
        ack = self.ok("submit", peer=CONTROLLER, **KEYBOARD_TYPE)
        self.restart()
        self.ready(instance="instance-2")
        self.assertIsNone(self.next())
        outcome = self.ok("outcome", peer=CONTROLLER, invocationId=ack["invocationId"])
        self.assertEqual(outcome["state"], "unknown")

    def test_clave_extra_se_rechaza(self):
        broken = json.loads(json.dumps(KEYBOARD_TYPE))
        broken["arguments"]["content"] = "colado"
        self.error("bad_argument", "submit", peer=CONTROLLER, **broken)

    def test_type_sin_texto_se_rechaza(self):
        broken = json.loads(json.dumps(KEYBOARD_TYPE))
        del broken["arguments"]["text"]
        self.error("missing_text", "submit", peer=CONTROLLER, **broken)

    def test_texto_con_salto_o_control_se_rechaza(self):
        for bad in ("a\nb", "a\tb", "a\x00b"):
            broken = json.loads(json.dumps(KEYBOARD_TYPE))
            broken["arguments"]["text"] = bad
            self.error("invalid_text", "submit", peer=CONTROLLER, **broken)

    def test_combo_valido_y_sin_combo(self):
        combo = {"submissionId": "submission-kbd2", "requestId": "request-1",
                 "sessionId": "session-1", "taskId": "task-2",
                 "toolId": "hefesto.keyboard",
                 "arguments": {"action": "combo", "combo": "cmd+s"}}
        self.ready()
        self.ok("submit", peer=CONTROLLER, **combo)
        self.assertEqual(self.next()["arguments"]["combo"], "cmd+s")
        broken = json.loads(json.dumps(combo))
        del broken["arguments"]["combo"]
        self.error("missing_combo", "submit", peer=CONTROLLER, **broken)

    def test_acepta_salida_de_texto_y_rechaza_info(self):
        self.ready()
        self.ok("submit", peer=CONTROLLER, **KEYBOARD_TYPE)
        invocation = self.next()
        self.ok("result", **self.result_fields(
            invocation, output={"text": "Escrito 7 caracteres en «TextEdit»"}))
        self.ok("submit", peer=CONTROLLER,
                **{**KEYBOARD_TYPE, "submissionId": "submission-kbd3", "taskId": "task-3"})
        second = self.next()
        self.error("invalid_text", "result", **self.result_fields(
            second, output={"exists": True, "size": 3, "sha256": "0" * 64}))
```

- [ ] **Step 2: Ejecutar para ver que falla**

Run: `cd server && python3 -m unittest test_hefesto -k keyboard`
Expected: FAIL.

- [ ] **Step 3: Implementar en el journal**

1. En la lista de `toolId` admitidos añade `"hefesto.keyboard"`.
2. Añade la rama de validación tras la de apps:

```python
        elif body["toolId"] == "hefesto.keyboard":
            if not set(args) <= {"action", "text", "combo"}:
                raise _Rejected("bad_argument", 400)
            action = args.get("action")
            if action not in ("type", "combo"):
                raise _Rejected("bad_action", 400)
            if action == "type":
                text = args.get("text")
                if type(text) is not str or not text:
                    raise _Rejected("missing_text", 400)
                try:
                    if (len(text.encode("utf-8")) > 4096
                            or any(unicodedata.category(c) == "Cc" for c in text)):
                        raise _Rejected("invalid_text", 400)
                except UnicodeError:
                    raise _Rejected("invalid_text", 400) from None
            else:
                combo = args.get("combo")
                if type(combo) is not str or not combo:
                    raise _Rejected("missing_combo", 400)
                try:
                    if len(combo.encode("utf-8")) > 64:
                        raise _Rejected("invalid_combo", 400)
                except UnicodeError:
                    raise _Rejected("invalid_combo", 400) from None
```

3. Redacción del submit (donde hoy se quita `content`):

```python
            hidden = {"content"}
            if data["toolId"] == "hefesto.keyboard":
                hidden.add("text")
            redacted["arguments"] = {k: v for k, v in arguments.items() if k not in hidden}
            if "content" in arguments:
                # ... como hoy (contentBytes/contentSha256)
            if data["toolId"] == "hefesto.keyboard" and "text" in arguments:
                encoded = arguments["text"].encode("utf-8")
                redacted["arguments"]["textBytes"] = len(encoded)
                redacted["arguments"]["textSha256"] = hashlib.sha256(encoded).hexdigest()
```

4. Columna `text` de la fila: guarda `""` para teclado:

```python
                stored_text = ("" if data["toolId"] == "hefesto.keyboard"
                               else data["arguments"].get("text", "")),
```
usada en el `INSERT` en lugar de `data["arguments"].get("text", "")`.

5. Contenido en memoria: la condición pasa a

```python
            if "content" in data["arguments"] or data["toolId"] == "hefesto.keyboard":
                self._contents[invocation_id] = data["arguments"]
```

6. En `next`, la rama de contenido en memoria pasa a cubrir ambos:

```python
                if submitted["toolId"] in ("hefesto.write_file", "hefesto.keyboard"):
```

7. En la validación de `output`, añade `hefesto.keyboard` a la rama que admite
   `_text_output` (como `rename`/`file_info`).
8. En `side_effects`, añade `"hefesto.keyboard"`.

- [ ] **Step 4: Ejecutar y pasar**

Run: `cd server && python3 -m unittest discover -p "test_*.py"`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add server/hefesto.py server/test_hefesto.py
git commit -m "feat(hefesto): journal v5 con teclado sin texto persistido"
```

---

### Task 6: Herramienta `hefesto_keyboard` en el relay

**Files:**
- Modify: `server/atlas_relay.py` (catálogo, resumen de tarjeta, handler, despacho, redacción de logs)
- Modify: `server/test_hefesto_delegation.py`

**Interfaces:**
- Consumes: `hefesto.keyboard` del journal (Task 5).
- Produces: herramienta `hefesto_keyboard` (`requiresConfirmation: True`) con `_keyboard(job, args) -> str`, tarjeta «Escribir «…» en la app en primer plano» / «Pulsar cmd+s en la app en primer plano», sin burbuja.

- [ ] **Step 1: Escribir los tests fallidos**

En `server/test_hefesto_delegation.py`, clase nueva `KeyboardFlowTests`:

```python
class KeyboardFlowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-kbd-")
        self.addCleanup(self.temp.cleanup)
        self.brain = make_brain(self.temp.name)
        self.journal = FakeJournal()
        self.brain._hefesto_journal = self.journal
        self.brain._hefesto_device = "mac-one"

    def test_catalogo_expone_teclado_con_tarjeta(self):
        tool = next(t for t in self.brain.tool_defs() if t["id"] == "hefesto_keyboard")
        self.assertTrue(tool["requiresConfirmation"])
        self.assertEqual(tool["schema"]["properties"]["action"]["enum"], ["type", "combo"])

    def test_type_pide_tarjeta_y_devuelve_resumen(self):
        self.journal.accept("inv-kbd")
        self.journal.outcomes["inv-kbd"] = {"state": "succeeded",
                                            "output": {"text": "Escrito 4 caracteres en «TextEdit»"}}
        with mock.patch.object(self.brain, "ask_confirmation", return_value="allow_once") as ask:
            result = self.brain.execute_tool(
                "hefesto_keyboard", {"action": "type", "text": "hola"}, JOB)
        self.assertEqual(result, "Escrito 4 caracteres en «TextEdit»")
        self.assertEqual(ask.call_args.kwargs["summary"],
                         ("Escribir «hola» en la app en primer plano", "write"))
        self.assertEqual(self.journal.submits[0]["toolId"], "hefesto.keyboard")
        self.assertEqual(self.journal.submits[0]["arguments"], {"action": "type", "text": "hola"})

    def test_combo_valido(self):
        self.journal.accept("inv-kbd")
        self.journal.outcomes["inv-kbd"] = {"state": "succeeded",
                                            "output": {"text": "Pulsado cmd+s en «TextEdit»"}}
        with mock.patch.object(self.brain, "ask_confirmation", return_value="allow_once"):
            result = self.brain.execute_tool("hefesto_keyboard",
                                             {"action": "combo", "combo": "cmd+s"}, JOB)
        self.assertEqual(result, "Pulsado cmd+s en «TextEdit»")

    def test_texto_con_salto_no_delega(self):
        with mock.patch.object(self.brain, "ask_confirmation") as ask:
            result = self.brain.execute_tool(
                "hefesto_keyboard", {"action": "type", "text": "a\nb"}, JOB)
        self.assertIn("texto no válido", result)
        ask.assert_not_called()
        self.assertEqual(self.journal.submits, [])

    def test_type_sin_texto_no_delega(self):
        with mock.patch.object(self.brain, "ask_confirmation") as ask:
            result = self.brain.execute_tool("hefesto_keyboard", {"action": "type"}, JOB)
        self.assertIn("falta text", result)
        ask.assert_not_called()

    def test_atajo_invalido_no_delega(self):
        with mock.patch.object(self.brain, "ask_confirmation") as ask:
            result = self.brain.execute_tool("hefesto_keyboard",
                                             {"action": "combo", "combo": "s"}, JOB)
        self.assertIn("atajo no válido", result)
        ask.assert_not_called()

    def test_sin_burbuja_hefesto(self):
        self.journal.accept("inv-kbd")
        self.journal.outcomes["inv-kbd"] = {"state": "succeeded",
                                            "output": {"text": "Escrito 4 caracteres en «TextEdit»"}}
        with mock.patch.object(self.brain, "ask_confirmation", return_value="allow_once"), \
                mock.patch.object(self.brain, "_hefesto_note") as note:
            self.brain.execute_tool("hefesto_keyboard",
                                    {"action": "type", "text": "hola"}, JOB)
        note.assert_not_called()

    def test_el_log_no_contiene_el_texto(self):
        self.journal.accept("inv-kbd")
        self.journal.outcomes["inv-kbd"] = {"state": "succeeded",
                                            "output": {"text": "Escrito 4 caracteres en «TextEdit»"}}
        call = {"id": "c1", "function": {"name": "hefesto_keyboard",
                                         "arguments": json.dumps({"action": "type", "text": "secreto"})}}
        with mock.patch.object(self.brain, "ask_confirmation", return_value="allow_once"), \
                mock.patch("atlas_relay.dlog") as log:
            self.brain._handle_tool_call(JOB, call)
        joined = " ".join(str(c) for c in log.call_args_list)
        self.assertNotIn("secreto", joined)
```

Ajusta `JOB`, `make_brain` y `FakeJournal` a los del fichero.

- [ ] **Step 2: Ejecutar para ver que falla**

Run: `cd server && python3 -m unittest test_hefesto_delegation -k Keyboard`
Expected: FAIL.

- [ ] **Step 3: Implementar**

1. Catálogo (`HEFESTO_KEYBOARD_TOOL`) con `requiresConfirmation: True`, `risk: "external"`,
   `action` enum `["type","combo"]`, `text` y `combo` opcionales, `required: ["action"]`.
   Añádelo a `HEFESTO_TOOLS`.
2. `_confirm_summary`: antes del `return` por defecto:

```python
        if tool_id == "hefesto_keyboard":
            if args.get("action") == "combo":
                return (f"Pulsar {args.get('combo', '?')} en la app en primer plano", "write")
            preview = str(args.get("text", ""))
            if len(preview) > 120:
                preview = preview[:120] + "…"
            return (f"Escribir «{preview}» en la app en primer plano", "write")
```

3. Constante `_KEYBOARD_COMBO_RE` a nivel de módulo:

```python
_KEYBOARD_COMBO_RE = re.compile(
    r"(?:(?:cmd|shift|alt|ctrl)\+){1,3}(?:[a-z0-9]|space|return|tab|esc|left|right|up|down|delete)")
```

4. Handler:

```python
    def _keyboard(self, job: dict, args: dict) -> str:
        """Teclado sintético en la app en primer plano; el texto nunca se registra."""
        action = args.get("action")
        if action not in ("type", "combo"):
            return "action inválida (type o combo)"
        if action == "type":
            text = args.get("text")
            if not isinstance(text, str) or not text:
                return "falta text para type"
            if (len(text.encode("utf-8")) > 4096
                    or any(not c.isprintable() for c in text)):
                return "texto no válido (sin saltos de línea ni controles)"
            arguments = {"action": "type", "text": text}
        else:
            combo = args.get("combo")
            if not isinstance(combo, str) or not combo:
                return "falta combo para combo"
            if len(combo.encode("utf-8")) > 64 or not _KEYBOARD_COMBO_RE.fullmatch(combo):
                return "atajo no válido"
            arguments = {"action": "combo", "combo": combo}
        invocation_id, error = self._hefesto_submit("hefesto.keyboard", job, arguments)
        if error is not None:
            return error
        output = self._hefesto_wait(invocation_id, job, 20.0)
        if isinstance(output, str):
            return output
        return str(output.get("text") or "")
```

5. Despacho en `execute_tool`: añade la rama `if name == "hefesto_keyboard": return self._keyboard(job, args)` junto a las demás `hefesto_*`.
6. Redacción de logs en `_handle_tool_call`:

```python
        logged = ("" if function.get("name") in ("hefesto_write_file", "hefesto_keyboard")
                  else str(function.get("arguments") or "")[:300])
```

- [ ] **Step 4: Ejecutar y pasar**

Run: `cd server && python3 -m unittest discover -p "test_*.py"`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add server/atlas_relay.py server/test_hefesto_delegation.py
git commit -m "feat(hefesto): herramienta hefesto_keyboard con tarjeta y sin texto en logs"
```

---

### Task 7: Órbita «Tecleando», contratos y plan

**Files:**
- Modify: `src/state/orbitPlan.ts` (`HEFESTO_PHASES`)
- Modify: `src/state/orbitPlan.test.ts`
- Modify: `docs/contracts.md` (sección Hefesto v4→v5, filas de `hefesto.keyboard`, `KeyboardPayload`, límites y tarjeta)
- Modify: `docs/hefesto-plan.md` (§5 v5 y §6 puerta 7 corte 2)

**Interfaces:**
- Consumes: relay emite `executing` con `message == "hefesto_keyboard"`.
- Produces: órbita violeta «Hefesto · Tecleando»; contrato documentado v5.

- [ ] **Step 1: Escribir el test de órbita fallido**

En `orbitPlan.test.ts`:

```ts
it('órbita violeta para teclear', () => {
  const plan = planPhaseOrbit('executing', 'hefesto_keyboard')
  expect(plan?.title).toBe('Hefesto · Tecleando')
  expect(plan?.color).toBe(HEFESTO_ORBIT_COLOR)
})
```

- [ ] **Step 2: Ejecutar para ver que falla**

Run: `npx vitest run src/state/orbitPlan.test.ts`
Expected: FAIL.

- [ ] **Step 3: Añadir la etiqueta**

En `HEFESTO_PHASES`: `hefesto_keyboard: 'Hefesto · Tecleando',`.

- [ ] **Step 4: Ejecutar y pasar**

Run: `npx vitest run src/state/orbitPlan.test.ts`
Expected: PASS.

- [ ] **Step 5: Actualizar `docs/contracts.md`**

Sube la sección a «Hefesto v5 — eco, lectura, escritura, apps y teclado», cambia todos
los `protocolVersion: 4` → `5` de la sección, añade la fila `hefesto.keyboard`
(`{action, text?, combo?}` → `{text}`) y un apartado con: límites del texto (≤4096 B,
sin controles), gramática del atajo (1–3 modificadores + tecla cerrada, al menos uno),
destino (solo app en primer plano), tarjeta siempre, entrada segura y permiso TCC,
y que el texto no se persiste, no se registra ni genera burbuja.

- [ ] **Step 6: Actualizar `docs/hefesto-plan.md`**

§5 cabecera a v5 y añade la línea de la corte 2 (teclado: `hefesto.keyboard`
type/combo, tarjeta siempre, CGEvent público, entrada segura, TCC, sin persistir
texto). En §6, puerta 7: marca «corte 2 — teclado» y deja ventanas, capturas y
multimedia como pendientes.

- [ ] **Step 7: Compilar y commit**

Run: `npm run build`
Expected: PASS.

```bash
git add src/state/orbitPlan.ts src/state/orbitPlan.test.ts docs/contracts.md docs/hefesto-plan.md
git commit -m "docs(hefesto): contrato v5 y órbita de teclado"
```

---

### Task 8: Verificación global, despliegue y prueba en vivo

**Files:**
- Modify: `docs/implemented.md` (entrada nueva con commits exactos)
- Modify: `docs/implementation-plan.md` (marcar el corte de teclado)
- Modify: `docs/hefesto-plan.md` §8 (registro de ejecución)

**Interfaces:**
- Consumes: Tasks 1‑7 terminadas y commiteadas.
- Produces: relay desplegado a v5, bundle reconstruido, matriz en vivo verde y registro actualizado.

- [ ] **Step 1: Suites completas**

Run: `cargo test --workspace`
Run: `cargo clippy --workspace --all-targets -- -D warnings`
Run: `cargo fmt --all -- --check`
Run: `cd server && python3 -m unittest discover -p "test_*.py"`
Run: `npm run test && npm run build && npm run lint`
Expected: todo verde; anota counts.

- [ ] **Step 2: Cobertura del spec**

Repasa §3.1‑3.6 y §5 del spec y señala qué tarea cubre cada requisito. Confirma:
una sola herramienta con tarjeta siempre, solo app en primer plano, texto sin
controles, atajo con modificador, entrada segura, TCC una vez por arranque, sin
persistencia ni log ni burbuja, puente en hilo principal.

- [ ] **Step 3: Bundle y despliegue (con permiso explícito del usuario)**

`set -a; . ./.env; set +a` y `npm run tauri build` con
`APPLE_SIGNING_IDENTITY="Apple Development: …"`; `codesign --verify --deep --strict`;
instalar con `ditto` en `/Applications/Atlas.app`. **Después** `./scripts/deploy-relay.sh`
(commitea antes si toca) y verifica `/v1/health`.

- [ ] **Step 4: Prueba en vivo con el usuario (con permiso)**

Matriz: escribir en TextEdit («Hola desde Hefesto», debe aparecer exacto); `cmd+a` en
TextEdit (selecciona todo); diálogo `osascript -e 'display dialog "prueba" default answer ""
with hidden answer giving up after 6'` en primer plano → «hay un campo con entrada
segura…»; sin permiso de Accesibilidad el primer uso muestra el diálogo del sistema;
tras concederlo, la escritura funciona. Comprobar que no quedan procesos nuevos ni RAM
añadida.

- [ ] **Step 5: Registro**

En `docs/implemented.md` nueva entrada con los commits exactos de este corte y la matriz
en vivo. En `docs/implementation-plan.md` marca «Teclado sintético (puerta 7, corte 2)».
En `docs/hefesto-plan.md` §8 añade la línea de ejecución con counts y despliegue.

- [ ] **Step 6: Commit final (con permiso)**

```bash
git add docs
git commit -m "docs(hefesto): registro del teclado sintético (puerta 7, corte 2)"
```
