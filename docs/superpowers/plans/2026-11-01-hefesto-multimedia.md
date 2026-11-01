# Multimedia de Hefesto Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hefesto sube/baja volumen y brillo, silencia y controla la reproducción con las teclas de sistema de macOS, con tarjeta de confirmación, protocolo v6 y sin leer ni exponer contenido multimedia.

**Architecture:** Una única herramienta de dispositivo `hefesto.media` (`{action, count?}`) y una única herramienta de relay `hefesto_media`. La lógica pura vive en `crates/atlas-mac/src/media.rs` detrás del trait `KeyboardBridge` ya existente, al que se añade `press_system`. El puente real (`src-tauri/src/hefesto_keyboard.rs`) postea eventos system-defined de AppKit (`NX_KEYTYPE_*`, subtipo 8, flag de función) al hilo principal. El journal trata el efecto como *sin contenido* (solo acción y count) y lo marca `unknown` si queda en vuelo tras un reinicio.

**Tech Stack:** Rust (atlas-mac, Tauri), CGEvent/AppKit vía `objc2` + `core-graphics`, Python (`server/hefesto.py`, `server/atlas_relay.py`), React/TypeScript (órbita).

**Spec:** `docs/superpowers/specs/2026-11-01-hefesto-multimedia-design.md`

## Global Constraints

- `protocolVersion` sube de 5 a 6 en los dos extremos y en todos los fixtures. Ambos lados rechazan versiones distintas.
- Sin subprocess, sin shell, sin API privada, sin permisos nuevos: se reutiliza el TCC de Accesibilidad ya concedido al teclado.
- Toda llamada válida a `hefesto_media` abre **exactamente una** tarjeta (`requiresConfirmation: True` + `planConfirmation: True`); con entrada inválida, **ninguna**. Sin burbuja `agent: hefesto`.
- Textos estables exactos (copiar tal cual, con guion largo `–` en `1–10`):
  - `action inválida (volume_up, volume_down, mute, brightness_up, brightness_down, play_pause, next o previous)`
  - `count fuera de rango (1–10)`
  - `count no aplica a esta acción`
  - `hay un campo con entrada segura en primer plano; no pulso teclas del sistema`
  - `falta el permiso de Accesibilidad; actívalo en Ajustes del Sistema › Privacidad y seguridad › Accesibilidad`
  - `no se pudo pulsar la tecla del sistema: {err}`
  - `Hefesto no puede controlar multimedia` (sin puente inyectado)
- Sin contenido multimedia: no se lee ni persiste estado de volumen, brillo, pista o app.
- `count` solo aplica a `volume_up`, `volume_down`, `brightness_up` y `brightness_down`; rango 1–10, por defecto 1.
- Nunca ejecutar `sudo`, no crear LaunchAgents, no dejar procesos huérfanos; las pruebas en vivo se hacen con el relay desplegado y el bundle nuevo.
- Commits por tarea, en español, con el estilo del repositorio.

---

### Task 1: Protocolo v5 → v6

**Files:**
- Modify: `crates/atlas-mac/src/wire.rs` (versión y test)
- Modify: `crates/atlas-mac/tests/wire.rs` (fixtures y nombres)
- Modify: `crates/atlas-mac/src/client.rs` (fixtures)
- Modify: `server/hefesto.py` (validación y respuestas)
- Modify: `server/atlas_relay.py` (solicitudes)
- Modify: `server/test_hefesto.py`, `server/test_hefesto_delegation.py` (fixtures)

**Interfaces:**
- Produces: `ProtocolVersion` que serializa/acepta únicamente `6`; el journal y el relay hablan v6.

- [ ] **Step 1: Reemplazar los literales JSON en los archivos afectados**

```bash
sed -i '' 's/"protocolVersion":5/"protocolVersion":6/g; s/"protocolVersion": 5/"protocolVersion": 6/g' \
  crates/atlas-mac/tests/wire.rs crates/atlas-mac/src/client.rs \
  server/hefesto.py server/atlas_relay.py server/test_hefesto.py server/test_hefesto_delegation.py
```

No tocar `server/test_hefesto_integration.py` (su `protocolVersion:2` es una prueba deliberada de rechazo).

- [ ] **Step 2: Ajustar los literales que no son JSON y el test de versión**

En `crates/atlas-mac/src/wire.rs`:
- `if value == 5 {` → `if value == 6 {`
- `impl From<ProtocolVersion> for u64 { ... 5 }` → `... 6 }`
- Renombrar `protocol_version_is_five` a `protocol_version_is_six` y dejar:

```rust
    #[test]
    fn protocol_version_is_six() {
        assert_eq!(u64::from(ProtocolVersion), 6);
        assert!(ProtocolVersion::try_from(6u64).is_ok());
        assert!(ProtocolVersion::try_from(5u64).is_err());
    }
```

En `server/hefesto.py` línea ~158: `body["protocolVersion"] != 5` → `!= 6`.
En `server/test_hefesto.py` línea ~73: `self.assertEqual(value["protocolVersion"], 5)` → `6`.
En `crates/atlas-mac/tests/wire.rs`: renombrar `teclado_v5_aterriza_en_su_variante_sin_robar_apps` a `teclado_v6_aterriza_en_su_variante_sin_robar_apps`.

- [ ] **Step 3: Ejecutar las suites**

Run: `cargo test -p atlas-mac`
Expected: PASS (todas las pruebas verdes).

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/atlas-mac/src/wire.rs crates/atlas-mac/tests/wire.rs crates/atlas-mac/src/client.rs \
  server/hefesto.py server/atlas_relay.py server/test_hefesto.py server/test_hefesto_delegation.py
git commit -m "feat(hefesto): protocolo v6 para multimedia"
```

---

### Task 2: Wire de media, módulo puro y puente de teclas de sistema

**Files:**
- Modify: `crates/atlas-mac/src/wire.rs` (acción, payload, ToolId, ToolArguments)
- Create: `crates/atlas-mac/src/media.rs`
- Modify: `crates/atlas-mac/src/keyboard.rs` (método del trait y fake)
- Modify: `crates/atlas-mac/src/lib.rs` (módulo y export)
- Modify: `crates/atlas-mac/tests/wire.rs` (prueba de variante)
- Modify: `src-tauri/Cargo.toml` (features y dependencia)
- Modify: `src-tauri/src/hefesto_keyboard.rs` (puente real)

**Interfaces:**
- Produces:
  - `wire::MediaAction` (`VolumeUp|VolumeDown|Mute|BrightnessUp|BrightnessDown|PlayPause|Next|Previous`, serde `snake_case`)
  - `wire::MediaPayload { action: MediaAction, count: Option<u8> }`
  - `wire::ToolId::Media` (`hefesto.media`) y `wire::ToolArguments::Media`
  - `media::SystemKey::nx_key_type() -> u32`
  - `media::validate_count(action, count) -> Result<u8, &'static str>`
  - `media::handle_media(&dyn KeyboardBridge, &MediaPayload) -> crate::Result<OutputPayload>`
  - `KeyboardBridge::press_system(&self, key: u32, count: u8) -> Result<(), String>`

- [ ] **Step 1: Añadir los tipos wire (y su prueba)**

En `crates/atlas-mac/src/wire.rs`, tras `KeyboardPayload`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaAction {
    VolumeUp,
    VolumeDown,
    Mute,
    BrightnessUp,
    BrightnessDown,
    PlayPause,
    Next,
    Previous,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaPayload {
    pub action: MediaAction,
    #[serde(default)]
    pub count: Option<u8>,
}
```

Añadir `Media(MediaPayload)` **al final** de `ToolArguments` (después de `Keyboard`) y `Media` a `ToolId`:

```rust
    #[serde(rename = "hefesto.media")]
    Media,
```

En `crates/atlas-mac/tests/wire.rs`:

```rust
#[test]
fn media_v6_aterriza_en_su_variante_sin_robar_teclado_ni_apps() {
    let volume: ToolArguments =
        serde_json::from_str(r#"{"action":"volume_up","count":3}"#).unwrap();
    assert_eq!(
        volume,
        ToolArguments::Media(MediaPayload {
            action: MediaAction::VolumeUp,
            count: Some(3),
        })
    );
    let typed: ToolArguments = serde_json::from_str(r#"{"action":"type","text":"hola"}"#).unwrap();
    assert!(matches!(typed, ToolArguments::Keyboard(_)));
    let apps: ToolArguments = serde_json::from_str(r#"{"action":"list"}"#).unwrap();
    assert!(matches!(apps, ToolArguments::Apps(_)));
}
```

Importar `MediaAction, MediaPayload` en el `use` de ese archivo.

- [ ] **Step 2: Ejecutar el test para ver que falla**

Run: `cargo test -p atlas-mac --test wire media_v6`
Expected: FAIL con errores de compilación (`Media`/`MediaPayload` no existen).

- [ ] **Step 3: Crear `crates/atlas-mac/src/media.rs`**

```rust
use crate::keyboard::{KeyboardBridge, NO_PERMISSION_TEXT};
use crate::wire::{MediaAction, MediaPayload, OutputPayload};

pub const MEDIA_SECURE_TEXT: &str =
    "hay un campo con entrada segura en primer plano; no pulso teclas del sistema";
pub const BAD_COUNT_RANGE_TEXT: &str = "count fuera de rango (1–10)";
pub const COUNT_NOT_APPLICABLE_TEXT: &str = "count no aplica a esta acción";

/// Teclas de sistema públicas de IOKit (`NX_KEYTYPE_*`, `ev_keymap.h`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemKey {
    SoundUp,
    SoundDown,
    BrightnessUp,
    BrightnessDown,
    Mute,
    PlayPause,
    Next,
    Previous,
}

impl SystemKey {
    pub fn nx_key_type(self) -> u32 {
        match self {
            Self::SoundUp => 0,
            Self::SoundDown => 1,
            Self::BrightnessUp => 2,
            Self::BrightnessDown => 3,
            Self::Mute => 7,
            Self::PlayPause => 16,
            Self::Next => 17,
            Self::Previous => 18,
        }
    }
}

impl MediaAction {
    pub fn system_key(self) -> SystemKey {
        match self {
            Self::VolumeUp => SystemKey::SoundUp,
            Self::VolumeDown => SystemKey::SoundDown,
            Self::Mute => SystemKey::Mute,
            Self::BrightnessUp => SystemKey::BrightnessUp,
            Self::BrightnessDown => SystemKey::BrightnessDown,
            Self::PlayPause => SystemKey::PlayPause,
            Self::Next => SystemKey::Next,
            Self::Previous => SystemKey::Previous,
        }
    }

    pub fn allows_count(self) -> bool {
        matches!(
            self,
            Self::VolumeUp | Self::VolumeDown | Self::BrightnessUp | Self::BrightnessDown
        )
    }
}

/// `count` solo vale para volumen y brillo: 1–10, por defecto 1.
pub fn validate_count(action: MediaAction, count: Option<u8>) -> Result<u8, &'static str> {
    if !action.allows_count() {
        return if count.is_some() {
            Err(COUNT_NOT_APPLICABLE_TEXT)
        } else {
            Ok(1)
        };
    }
    match count.unwrap_or(1) {
        1..=10 => Ok(count.unwrap_or(1)),
        _ => Err(BAD_COUNT_RANGE_TEXT),
    }
}

fn steps(count: u8) -> String {
    if count == 1 {
        "1 paso".to_string()
    } else {
        format!("{count} pasos")
    }
}

pub fn summary(action: MediaAction, count: u8) -> String {
    match action {
        MediaAction::VolumeUp => format!("Subido {} de volumen", steps(count)),
        MediaAction::VolumeDown => format!("Bajado {} de volumen", steps(count)),
        MediaAction::BrightnessUp => format!("Subido {} de brillo", steps(count)),
        MediaAction::BrightnessDown => format!("Bajado {} de brillo", steps(count)),
        MediaAction::Mute => "Pulsada la tecla de silencio".to_string(),
        MediaAction::PlayPause => "Pulsada la tecla de reproducción/pausa".to_string(),
        MediaAction::Next => "Pulsada la tecla de siguiente pista".to_string(),
        MediaAction::Previous => "Pulsada la tecla de pista anterior".to_string(),
    }
}

pub fn handle_media(
    bridge: &dyn KeyboardBridge,
    payload: &MediaPayload,
) -> crate::Result<OutputPayload> {
    let count = match validate_count(payload.action, payload.count) {
        Ok(count) => count,
        Err(text) => return OutputPayload::text(text),
    };
    if bridge.secure_input() {
        return OutputPayload::text(MEDIA_SECURE_TEXT);
    }
    if !bridge.can_post(true) {
        return OutputPayload::text(NO_PERMISSION_TEXT);
    }
    let key = payload.action.system_key();
    match bridge.press_system(key.nx_key_type(), count) {
        Ok(()) => OutputPayload::text(summary(payload.action, count)),
        Err(err) => OutputPayload::text(format!("no se pudo pulsar la tecla del sistema: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

    struct FakeKeyboard {
        granted: bool,
        secure: bool,
        fail: bool,
        called: AtomicBool,
        prompted: AtomicBool,
        key: AtomicU32,
        presses: AtomicU8,
    }

    impl FakeKeyboard {
        fn ok() -> Self {
            Self {
                granted: true,
                secure: false,
                fail: false,
                called: AtomicBool::new(false),
                prompted: AtomicBool::new(false),
                key: AtomicU32::new(u32::MAX),
                presses: AtomicU8::new(0),
            }
        }
    }

    impl KeyboardBridge for FakeKeyboard {
        fn can_post(&self, prompt: bool) -> bool {
            if prompt {
                self.prompted.store(true, Ordering::SeqCst);
            }
            self.granted
        }

        fn secure_input(&self) -> bool {
            self.secure
        }

        fn frontmost_app(&self) -> Option<String> {
            None
        }

        fn type_text(&self, _: &str) -> Result<(), String> {
            Ok(())
        }

        fn press(&self, _: u8, _: u16) -> Result<(), String> {
            Ok(())
        }

        fn press_system(&self, key: u32, count: u8) -> Result<(), String> {
            self.called.store(true, Ordering::SeqCst);
            self.key.store(key, Ordering::SeqCst);
            self.presses.store(count, Ordering::SeqCst);
            if self.fail {
                return Err("sin foco".to_string());
            }
            Ok(())
        }
    }

    fn payload(action: MediaAction, count: Option<u8>) -> MediaPayload {
        MediaPayload { action, count }
    }

    fn out(bridge: &FakeKeyboard, payload: &MediaPayload) -> String {
        handle_media(bridge, payload)
            .unwrap()
            .text_value()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn cada_accion_mapea_su_tecla_nx() {
        let cases = [
            (MediaAction::VolumeUp, 0),
            (MediaAction::VolumeDown, 1),
            (MediaAction::BrightnessUp, 2),
            (MediaAction::BrightnessDown, 3),
            (MediaAction::Mute, 7),
            (MediaAction::PlayPause, 16),
            (MediaAction::Next, 17),
            (MediaAction::Previous, 18),
        ];
        for (action, key) in cases {
            assert_eq!(action.system_key().nx_key_type(), key);
        }
    }

    #[test]
    fn count_por_defecto_uno_y_rango_1_10() {
        assert_eq!(validate_count(MediaAction::VolumeUp, None), Ok(1));
        assert_eq!(validate_count(MediaAction::VolumeUp, Some(1)), Ok(1));
        assert_eq!(validate_count(MediaAction::BrightnessDown, Some(10)), Ok(10));
        assert_eq!(
            validate_count(MediaAction::VolumeUp, Some(0)),
            Err(BAD_COUNT_RANGE_TEXT)
        );
        assert_eq!(
            validate_count(MediaAction::VolumeUp, Some(11)),
            Err(BAD_COUNT_RANGE_TEXT)
        );
    }

    #[test]
    fn count_no_aplica_a_mute_ni_reproduccion() {
        assert_eq!(validate_count(MediaAction::Mute, None), Ok(1));
        assert_eq!(
            validate_count(MediaAction::Mute, Some(1)),
            Err(COUNT_NOT_APPLICABLE_TEXT)
        );
        assert_eq!(
            validate_count(MediaAction::PlayPause, Some(2)),
            Err(COUNT_NOT_APPLICABLE_TEXT)
        );
    }

    #[test]
    fn entrada_invalida_no_toca_al_puente() {
        let bridge = FakeKeyboard::ok();
        assert_eq!(
            out(&bridge, &payload(MediaAction::VolumeUp, Some(11))),
            BAD_COUNT_RANGE_TEXT
        );
        assert_eq!(
            out(&bridge, &payload(MediaAction::Mute, Some(1))),
            COUNT_NOT_APPLICABLE_TEXT
        );
        assert!(!bridge.called.load(Ordering::SeqCst));
        assert!(!bridge.prompted.load(Ordering::SeqCst));
    }

    #[test]
    fn entrada_segura_no_pulsa() {
        let bridge = FakeKeyboard {
            secure: true,
            ..FakeKeyboard::ok()
        };
        assert_eq!(
            out(&bridge, &payload(MediaAction::VolumeUp, None)),
            MEDIA_SECURE_TEXT
        );
        assert!(!bridge.called.load(Ordering::SeqCst));
    }

    #[test]
    fn sin_permiso_pide_una_vez_y_no_pulsa() {
        let bridge = FakeKeyboard {
            granted: false,
            ..FakeKeyboard::ok()
        };
        assert_eq!(
            out(&bridge, &payload(MediaAction::PlayPause, None)),
            NO_PERMISSION_TEXT
        );
        assert!(bridge.prompted.load(Ordering::SeqCst));
        assert!(!bridge.called.load(Ordering::SeqCst));
    }

    #[test]
    fn volumen_repite_count_veces_y_resume_pasos() {
        let bridge = FakeKeyboard::ok();
        assert_eq!(
            out(&bridge, &payload(MediaAction::VolumeUp, Some(3))),
            "Subido 3 pasos de volumen"
        );
        assert_eq!(bridge.key.load(Ordering::SeqCst), 0);
        assert_eq!(bridge.presses.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn brillo_y_silencio_resumen_sin_afirmar_estado() {
        let bridge = FakeKeyboard::ok();
        assert_eq!(
            out(&bridge, &payload(MediaAction::BrightnessDown, Some(1))),
            "Bajado 1 paso de brillo"
        );
        assert_eq!(
            out(&bridge, &payload(MediaAction::Mute, None)),
            "Pulsada la tecla de silencio"
        );
        assert_eq!(
            out(&bridge, &payload(MediaAction::Previous, None)),
            "Pulsada la tecla de pista anterior"
        );
        assert_eq!(bridge.presses.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn error_del_puente_es_estable() {
        let bridge = FakeKeyboard {
            fail: true,
            ..FakeKeyboard::ok()
        };
        assert_eq!(
            out(&bridge, &payload(MediaAction::Next, None)),
            "no se pudo pulsar la tecla del sistema: sin foco"
        );
    }
}
```

- [ ] **Step 4: Extender `KeyboardBridge` con `press_system`**

En `crates/atlas-mac/src/keyboard.rs`, añadir al trait:

```rust
    fn press_system(&self, key: u32, count: u8) -> Result<(), String>;
```

Y en el `FakeKeyboard` de sus pruebas:

```rust
        fn press_system(&self, _: u32, _: u8) -> Result<(), String> {
            self.called.store(true, Ordering::SeqCst);
            Ok(())
        }
```

En `crates/atlas-mac/src/lib.rs`:

```rust
pub mod media;
...
pub use media::handle_media;
```

- [ ] **Step 5: Implementar el puente real de teclas de sistema**

En `src-tauri/Cargo.toml`, dentro de `[target.'cfg(target_os = "macos")'.dependencies]`:

```toml
objc2-app-kit = { version = "0.3", features = ["NSWorkspace", "NSRunningApplication", "NSEvent", "NSGraphicsContext", "objc2-core-graphics"] }
objc2-core-graphics = { version = "0.3", features = ["CGEvent", "CGEventTypes"] }
```

(`core-graphics = "0.24"` se conserva.)

En `src-tauri/src/hefesto_keyboard.rs`, imports y código:

```rust
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use objc2_core_graphics::{CGEventPost, CGEventTapLocation};
use objc2_foundation::NSPoint;
```

```rust
/// Bits públicos de estado de una tecla de sistema (`NX_KEYDOWN`/`NX_KEYUP`).
const NX_KEYDOWN: isize = 0x0A00;
const NX_KEYUP: isize = 0x0B00;
/// `NX_SUBTYPE_AUX_CONTROL_BUTTONS` de `ev_keymap.h`.
const NX_SUBTYPE_AUX_CONTROL_BUTTONS: i16 = 8;
const SYSTEM_EVENT_ERROR: &str = "no se pudo crear el evento de tecla del sistema";

fn system_key_data1(key: u32, down: bool) -> isize {
    ((key as isize) << 16) | if down { NX_KEYDOWN } else { NX_KEYUP }
}

/// Postea `count` pulsaciones (down+up) de una tecla de sistema por el tap HID.
fn post_system_on_main(key: u32, count: u8) -> Result<(), String> {
    for _ in 0..count {
        for down in [true, false] {
            let event = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
                NSEventType::SystemDefined,
                NSPoint::ZERO,
                NSEventModifierFlags::Function,
                0.0,
                0,
                None,
                NX_SUBTYPE_AUX_CONTROL_BUTTONS,
                system_key_data1(key, down),
                -1,
            )
            .ok_or_else(|| SYSTEM_EVENT_ERROR.to_string())?;
            let cg = event
                .CGEvent()
                .ok_or_else(|| SYSTEM_EVENT_ERROR.to_string())?;
            CGEventPost(CGEventTapLocation::HIDEventTap, Some(&cg));
        }
    }
    Ok(())
}
```

Añadir al `impl KeyboardBridge for CgKeyboard`:

```rust
    fn press_system(&self, key: u32, count: u8) -> Result<(), String> {
        self.on_main(move || post_system_on_main(key, count))?
    }
```

Y una prueba pura al final del módulo:

```rust
    #[test]
    fn data1_empaqueta_tecla_y_estado() {
        assert_eq!(system_key_data1(0, true), 0x0A00);
        assert_eq!(system_key_data1(0, false), 0x0B00);
        assert_eq!(system_key_data1(16, true), (16 << 16) | 0x0A00);
        assert_eq!(system_key_data1(18, false), (18 << 16) | 0x0B00);
    }
```

- [ ] **Step 6: Compilar y probar**

Run: `cargo test -p atlas-mac`
Expected: PASS, incluida `cada_accion_mapea_su_tecla_nx`.

Run: `cargo check --workspace`
Expected: PASS (el puente real compila con las features nuevas).

Run: `cargo fmt --check && cargo clippy -p atlas-mac --all-targets -- -D warnings`
Expected: limpio.

- [ ] **Step 7: Commit**

```bash
git add crates/atlas-mac/src/wire.rs crates/atlas-mac/src/media.rs crates/atlas-mac/src/keyboard.rs \
  crates/atlas-mac/src/lib.rs crates/atlas-mac/tests/wire.rs src-tauri/Cargo.toml src-tauri/Cargo.lock \
  src-tauri/src/hefesto_keyboard.rs
git commit -m "feat(hefesto): media pura y puente de teclas de sistema"
```

---

### Task 3: Despacho `hefesto.media` en el ejecutor

**Files:**
- Modify: `crates/atlas-mac/src/executor.rs`
- Modify: `crates/atlas-mac/tests/executor.rs`

**Interfaces:**
- Consumes: `media::handle_media`, `wire::ToolId::Media`, `wire::ToolArguments::Media`, `KeyboardBridge::press_system`.
- Produces: `Executor::execute` enruta `(ToolId::Media, ToolArguments::Media)` al `keyboard_bridge`; sin puente devuelve el texto `Hefesto no puede controlar multimedia`.

- [ ] **Step 1: Escribir los tests del ejecutor**

En `crates/atlas-mac/tests/executor.rs`, extender `FakeKeyboard` con:

```rust
    fn press_system(&self, _: u32, _: u8) -> std::result::Result<(), String> {
        Ok(())
    }
```

Añadir el helper y dos pruebas al final:

```rust
fn media_invocation(action: MediaAction, count: Option<u8>, inv_id: &str) -> Invocation {
    let mut inv = invocation();
    inv.invocation_id = id(inv_id);
    inv.tool_id = ToolId::Media;
    inv.arguments = ToolArguments::Media(MediaPayload { action, count });
    inv
}

#[test]
fn media_se_enruta_por_el_puente_de_teclado_inyectado() {
    let mut executor = keyboard_executor();
    let out = executor
        .execute(
            &media_invocation(MediaAction::VolumeUp, Some(2), "m-volume"),
            at(3),
        )
        .unwrap();
    assert_eq!(out.output.text_value(), Some("Subido 2 pasos de volumen"));
}

#[test]
fn media_sin_puente_devuelve_texto_no_disponible() {
    let mut executor = apps_executor(None);
    let out = executor
        .execute(
            &media_invocation(MediaAction::Mute, None, "m-none"),
            at(3),
        )
        .unwrap();
    assert_eq!(
        out.output.text_value(),
        Some("Hefesto no puede controlar multimedia")
    );
}
```

Importar `MediaAction, MediaPayload` en el `use` del archivo.

- [ ] **Step 2: Ejecutar para ver que falla**

Run: `cargo test -p atlas-mac --test executor media_`
Expected: FAIL (`media_se_enruta...` devuelve `Err(InvalidArguments)` y el segundo no compila la expectativa).

- [ ] **Step 3: Añadir el brazo al despachador**

En `crates/atlas-mac/src/executor.rs`, tras el brazo de `Keyboard`:

```rust
            (ToolId::Media, ToolArguments::Media(payload)) => match &self.keyboard_bridge {
                Some(bridge) => crate::media::handle_media(bridge.as_ref(), payload)?,
                None => OutputPayload::text("Hefesto no puede controlar multimedia")?,
            },
```

- [ ] **Step 4: Ejecutar las pruebas**

Run: `cargo test -p atlas-mac`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/atlas-mac/src/executor.rs crates/atlas-mac/tests/executor.rs
git commit -m "feat(hefesto): despacho de hefesto.media"
```

---

### Task 4: Journal v6 con `hefesto.media`

**Files:**
- Modify: `server/hefesto.py`
- Modify: `server/test_hefesto.py`

**Interfaces:**
- Consumes: protocolo v6 de T1.
- Produces: `submit` acepta `hefesto.media` con `arguments ⊆ {action, count}`; acción cerrada; `count` entero 1–10 solo para volumen/brillo; el efecto queda `unknown` si hay reinicio con la invocación pendiente; sin contenido en `_contents`.

- [ ] **Step 1: Escribir los tests del journal**

En `server/test_hefesto.py`, añadir al final, junto a `AppControlJournalTests`:

```python
MEDIA = {"submissionId": "submission-m", "requestId": "request-1", "sessionId": "session-1",
         "taskId": "task-m", "toolId": "hefesto.media",
         "arguments": {"action": "volume_up", "count": 2}}


class MediaJournalTests(JournalTests):
    @staticmethod
    def media(arguments, **changes):
        return {**MEDIA, **changes, "arguments": arguments}

    def test_media_submit_next_result(self):
        self.ready()
        ack = self.ok("submit", peer=CONTROLLER, **MEDIA)
        invocation = self.next()
        self.assertEqual(invocation["invocationId"], ack["invocationId"])
        self.assertEqual(invocation["toolId"], "hefesto.media")
        self.assertEqual(invocation["arguments"], {"action": "volume_up", "count": 2})
        result = self.ok("result", **self.result_fields(
            invocation, output={"text": "Subido 2 pasos de volumen"}))
        self.assertEqual(result["state"], "succeeded")

    def test_media_action_invalida_se_rechaza(self):
        self.ready()
        self.error("bad_action", "submit", peer=CONTROLLER,
                   **self.media({"action": "explode"}))

    def test_media_count_fuera_de_rango_se_rechaza(self):
        self.ready()
        for count in (0, 11, "2", True):
            self.error("bad_count", "submit", peer=CONTROLLER,
                       **self.media({"action": "volume_down", "count": count},
                                    submissionId=f"submission-mc-{count}",
                                    taskId=f"task-mc-{count}"))

    def test_media_count_no_aplica_se_rechaza(self):
        self.ready()
        self.error("bad_count", "submit", peer=CONTROLLER,
                   **self.media({"action": "mute", "count": 1},
                                submissionId="submission-mm", taskId="task-mm"))

    def test_media_claves_extra_se_rechazan(self):
        self.ready()
        self.error("bad_argument", "submit", peer=CONTROLLER,
                   **self.media({"action": "mute", "text": "x"},
                                submissionId="submission-mx", taskId="task-mx"))

    def test_reinicio_pendiente_de_media_queda_unknown_y_no_reenvia(self):
        self.ready()
        ack = self.ok("submit", peer=CONTROLLER, **MEDIA)
        self.restart()
        self.ready(instance="instance-2")
        self.assertIsNone(self.next())
        outcome = self.ok("outcome", peer=CONTROLLER, invocationId=ack["invocationId"])
        self.assertEqual(outcome["state"], "unknown")

    def test_media_rechaza_output_malformado(self):
        self.ready()
        self.ok("submit", peer=CONTROLLER, **MEDIA)
        invocation = self.next()
        self.error("invalid_text", "result", **self.result_fields(
            invocation, output={"volume": 2}))
```

- [ ] **Step 2: Ejecutar para ver que fallan**

Run: `python3 -m unittest discover -s server -p 'test_hefesto.py' -q`
Expected: FAIL (`unsupported_tool` en los tests de media).

- [ ] **Step 3: Implementar la validación y el efecto**

En `server/hefesto.py`, junto a las constantes de módulo:

```python
_MEDIA_ACTIONS = ("volume_up", "volume_down", "mute", "brightness_up",
                  "brightness_down", "play_pause", "next", "previous")
_MEDIA_COUNT_ACTIONS = ("volume_up", "volume_down", "brightness_up", "brightness_down")
```

Añadir `"hefesto.media"` a la tupla de `toolId` aceptados en `_validate`.

Añadir la rama, entre la de `hefesto.keyboard` y el `else`:

```python
        elif body["toolId"] == "hefesto.media":
            if not set(args) <= {"action", "count"}:
                raise _Rejected("bad_argument", 400)
            action = args.get("action")
            if action not in _MEDIA_ACTIONS:
                raise _Rejected("bad_action", 400)
            if "count" in args:
                count = args["count"]
                if type(count) is not int or not 1 <= count <= 10:
                    raise _Rejected("bad_count", 400)
                if action not in _MEDIA_COUNT_ACTIONS:
                    raise _Rejected("bad_count", 400)
```

En `Journal.__init__`, añadir `"hefesto.media"` a `side_effects`:

```python
            side_effects = ("hefesto.write_file", "hefesto.apps", "hefesto.keyboard",
                            "hefesto.media")
```

No se toca `_contents` ni la redacción: media no transporta contenido.

- [ ] **Step 4: Ejecutar las pruebas**

Run: `python3 -m unittest discover -s server -p 'test_hefesto.py' -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add server/hefesto.py server/test_hefesto.py
git commit -m "feat(hefesto): journal acepta hefesto.media como efecto sin contenido"
```

---

### Task 5: Herramienta `hefesto_media` en el relay

**Files:**
- Modify: `server/atlas_relay.py`
- Modify: `server/test_hefesto_delegation.py`

**Interfaces:**
- Consumes: `hefesto.media` de T4 y textos estables del spec.
- Produces: una herramienta `hefesto_media` (`requiresConfirmation: True`, `planConfirmation: True`, riesgo `write`), validación antes de la tarjeta, una tarjeta por llamada válida, cero con entrada inválida, sin burbuja.

- [ ] **Step 1: Escribir los tests del relay**

Añadir al final de `server/test_hefesto_delegation.py`:

```python
class MediaFlowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-media-")
        self.addCleanup(self.temp.cleanup)
        self.brain = make_brain(self.temp.name)
        self.journal = FakeJournal()
        self.brain._hefesto_journal = self.journal
        self.brain._hefesto_device = "mac-one"

    def test_catalogo_expone_media_con_tarjeta(self):
        tool = next(t for t in self.brain.tool_defs() if t["id"] == "hefesto_media")
        self.assertTrue(tool["requiresConfirmation"])
        self.assertTrue(tool["planConfirmation"])
        self.assertEqual(tool["schema"]["properties"]["action"]["enum"],
                         ["volume_up", "volume_down", "mute", "brightness_up",
                          "brightness_down", "play_pause", "next", "previous"])
        count = tool["schema"]["properties"]["count"]
        self.assertEqual((count["minimum"], count["maximum"]), (1, 10))

    def test_volumen_pide_tarjeta_y_normaliza_count(self):
        self.journal.accept("inv-media")
        self.journal.outcomes["inv-media"] = {
            "state": "succeeded", "output": {"text": "Subido 3 pasos de volumen"}}
        with mock.patch.object(self.brain, "ask_confirmation", return_value="allow_once") as ask:
            result = self.brain.execute_tool(
                "hefesto_media", {"action": "volume_up", "count": 3}, JOB)
        self.assertEqual(result, "Subido 3 pasos de volumen")
        self.assertEqual(ask.call_args.kwargs["summary"],
                         ("Subir el volumen 3 pasos", "write"))
        self.assertEqual(self.journal.submits[0]["toolId"], "hefesto.media")
        self.assertEqual(self.journal.submits[0]["arguments"],
                         {"action": "volume_up", "count": 3})

    def test_media_sin_count_usa_uno(self):
        self.journal.accept("inv-media")
        self.journal.outcomes["inv-media"] = {
            "state": "succeeded", "output": {"text": "Bajado 1 paso de brillo"}}
        with mock.patch.object(self.brain, "ask_confirmation", return_value="allow_once"):
            self.brain.execute_tool("hefesto_media", {"action": "brightness_down"}, JOB)
        self.assertEqual(self.journal.submits[0]["arguments"],
                         {"action": "brightness_down", "count": 1})

    def test_mute_no_lleva_count(self):
        self.journal.accept("inv-media")
        self.journal.outcomes["inv-media"] = {
            "state": "succeeded", "output": {"text": "Pulsada la tecla de silencio"}}
        with mock.patch.object(self.brain, "ask_confirmation", return_value="allow_once"):
            self.brain.execute_tool("hefesto_media", {"action": "mute"}, JOB)
        self.assertEqual(self.journal.submits[0]["arguments"], {"action": "mute"})

    def test_action_invalida_no_delega(self):
        with mock.patch.object(self.brain, "ask_confirmation") as ask:
            result = self.brain.execute_tool("hefesto_media", {"action": "explode"}, JOB)
        self.assertEqual(
            result,
            "action inválida (volume_up, volume_down, mute, brightness_up, "
            "brightness_down, play_pause, next o previous)")
        ask.assert_not_called()
        self.assertEqual(self.journal.submits, [])

    def test_count_invalido_no_delega(self):
        cases = (
            ({"action": "volume_up", "count": 0}, "count fuera de rango (1–10)"),
            ({"action": "volume_up", "count": 11}, "count fuera de rango (1–10)"),
            ({"action": "volume_up", "count": "2"}, "count fuera de rango (1–10)"),
            ({"action": "mute", "count": 1}, "count no aplica a esta acción"),
            ({"action": "next", "count": 2}, "count no aplica a esta acción"),
        )
        for args, expected in cases:
            with mock.patch.object(self.brain, "ask_confirmation") as ask:
                self.assertEqual(self.brain.execute_tool("hefesto_media", args, JOB), expected)
            ask.assert_not_called()
        self.assertEqual(self.journal.submits, [])

    def test_sin_burbuja_hefesto(self):
        self.journal.accept("inv-media")
        self.journal.outcomes["inv-media"] = {
            "state": "succeeded", "output": {"text": "Pulsada la tecla de silencio"}}
        with mock.patch.object(self.brain, "ask_confirmation", return_value="allow_once"), \
                mock.patch.object(self.brain, "_hefesto_note") as note:
            self.brain.execute_tool("hefesto_media", {"action": "mute"}, JOB)
        note.assert_not_called()

    def test_handle_tool_call_abre_una_sola_tarjeta(self):
        self.journal.accept("inv-media")
        self.journal.outcomes["inv-media"] = {
            "state": "succeeded", "output": {"text": "Pulsada la tecla de reproducción/pausa"}}
        call = {"id": "c1", "function": {"name": "hefesto_media",
                                         "arguments": json.dumps({"action": "play_pause"})}}
        with mock.patch.object(self.brain, "ask_confirmation", return_value="allow_once") as ask, \
                mock.patch("atlas_relay.dlog"):
            self.brain._handle_tool_call(JOB, call)
        self.assertEqual(ask.call_count, 1)

    def test_entrada_invalida_no_abre_tarjeta(self):
        call = {"id": "c1", "function": {"name": "hefesto_media",
                                         "arguments": json.dumps({"action": "mute", "count": 1})}}
        with mock.patch.object(self.brain, "ask_confirmation") as ask, \
                mock.patch("atlas_relay.dlog"):
            result = self.brain._handle_tool_call(JOB, call)
        self.assertEqual(result, "count no aplica a esta acción")
        ask.assert_not_called()
        self.assertEqual(self.journal.submits, [])
```

- [ ] **Step 2: Ejecutar para ver que fallan**

Run: `python3 -m unittest discover -s server -p 'test_hefesto_delegation.py' -q`
Expected: FAIL (`StopIteration` al buscar `hefesto_media`).

- [ ] **Step 3: Añadir la herramienta al catálogo**

En `server/atlas_relay.py`, tras `HEFESTO_KEYBOARD_TOOL`:

```python
HEFESTO_MEDIA_TOOL = {
    "id": "hefesto_media",
    "risk": "external",
    "requiresConfirmation": True,
    # La tarjeta la abre _media después de validar; _handle_tool_call no abre otra.
    "planConfirmation": True,
    "description": ("Sube o baja el volumen o el brillo, silencia y controla la reproducción "
                    "del Mac con Hefesto usando las teclas de sistema. `count` (1–10) solo "
                    "aplica a volumen y brillo (por defecto 1). Cada pulsación pide confirmación."),
    "schema": {
        "type": "object",
        "properties": {
            "action": {"type": "string",
                       "enum": ["volume_up", "volume_down", "mute", "brightness_up",
                                "brightness_down", "play_pause", "next", "previous"],
                       "description": "acción multimedia"},
            "count": {"type": "integer", "minimum": 1, "maximum": 10,
                      "description": "pulsaciones (solo volumen y brillo; por defecto 1)"},
        },
        "required": ["action"],
    },
}

HEFESTO_TOOLS = [HEFESTO_TOOL, HEFESTO_LIST_TOOL, HEFESTO_READ_TOOL,
                 HEFESTO_WRITE_TOOL, HEFESTO_RENAME_TOOL, HEFESTO_APPS_TOOL,
                 HEFESTO_KEYBOARD_TOOL, HEFESTO_MEDIA_TOOL]
```

- [ ] **Step 4: Enrutar y ejecutar la herramienta**

En `execute_tool`, tras `hefesto_keyboard`:

```python
        if tool_id == "hefesto_media":
            return self._media(job, args)
```

Añadir el método después de `_keyboard`:

```python
    def _media(self, job: dict, args: dict) -> str:
        """Teclas de sistema (volumen, brillo, reproducción); siempre con tarjeta."""
        action = args.get("action")
        if action not in ("volume_up", "volume_down", "mute", "brightness_up",
                          "brightness_down", "play_pause", "next", "previous"):
            return ("action inválida (volume_up, volume_down, mute, brightness_up, "
                    "brightness_down, play_pause, next o previous)")
        counted = action in ("volume_up", "volume_down", "brightness_up", "brightness_down")
        if counted:
            count = args.get("count", 1)
            if type(count) is not int or not 1 <= count <= 10:
                return "count fuera de rango (1–10)"
            arguments = {"action": action, "count": count}
        else:
            if "count" in args:
                return "count no aplica a esta acción"
            arguments = {"action": action}
        # El tiempo del usuario no consume el plazo de la operación.
        job["deadline"] = time.monotonic() + CONFIRM_TIMEOUT_S + 60
        summary = self._confirm_summary("hefesto_media", arguments)
        if self.ask_confirmation(job["request_id"], job["session_id"],
                                 "hefesto_media", arguments,
                                 chat_id=job.get("chat_id"), summary=summary) != "allow_once":
            return "denegada por el usuario"
        invocation_id, error = self._hefesto_submit("hefesto.media", job, arguments)
        if error is not None:
            return error
        output = self._hefesto_wait(invocation_id, job, 20.0)
        if isinstance(output, str):
            return output
        return str(output.get("text") or "")
```

- [ ] **Step 5: Añadir el resumen de la tarjeta**

En `_confirm_summary`, antes del `return` final:

```python
        if tool_id == "hefesto_media":
            action = str(args.get("action", "?"))
            count = args.get("count", 1)
            extra = f" {count} pasos" if counted_steps(count) else ""
            labels = {
                "volume_up": f"Subir el volumen{extra}",
                "volume_down": f"Bajar el volumen{extra}",
                "mute": "Silenciar o reactivar el sonido",
                "brightness_up": f"Subir el brillo{extra}",
                "brightness_down": f"Bajar el brillo{extra}",
                "play_pause": "Reproducir o pausar",
                "next": "Pasar a la siguiente pista",
                "previous": "Volver a la pista anterior",
            }
            return (labels.get(action, f"Multimedia: {action}"), "write")
```

con el helper de módulo, junto a `norm_name`:

```python
def counted_steps(count) -> bool:
    return type(count) is int and count > 1
```

- [ ] **Step 6: Ejecutar las pruebas del relay**

Run: `python3 -m unittest discover -s server -p 'test_hefesto_delegation.py' -q`
Expected: PASS.

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add server/atlas_relay.py server/test_hefesto_delegation.py
git commit -m "feat(hefesto): herramienta hefesto_media con tarjeta y sin burbuja"
```

---

### Task 6: Órbita, contratos y plan

**Files:**
- Modify: `src/state/orbitPlan.ts`, `src/state/orbitPlan.test.ts`
- Modify: `docs/hefesto-plan.md`
- Modify: `docs/contracts.md`

**Interfaces:**
- Consumes: `hefesto_media` del catálogo.
- Produces: órbita `hefesto_media` → «Hefesto · Multimedia»; §5/§6 del plan y contratos alineados con v6.

- [ ] **Step 1: Añadir la órbita y su prueba**

En `src/state/orbitPlan.ts`:

```ts
  hefesto_media: 'Hefesto · Multimedia',
```

En `src/state/orbitPlan.test.ts`, junto a la prueba del teclado:

```ts
  it('órbita violeta para multimedia', () => {
    const plan = planPhaseOrbit('executing', 'hefesto_media')
    expect(plan).toEqual({
      key: 'executing:hefesto_media',
      title: 'Hefesto · Multimedia',
      detail: '',
      color: HEFESTO_ORBIT_COLOR,
    })
  })
```

- [ ] **Step 2: Ejecutar las pruebas del frontend**

Run: `npx vitest run src/state/orbitPlan.test.ts`
Expected: PASS.

- [ ] **Step 3: Actualizar el plan de Hefesto**

En `docs/hefesto-plan.md`:
- Título de §5: `## 5. Protocolo v6 (eco, lectura, escritura, apps, teclado y multimedia)`.
- Sustituir en §5 las menciones `protocolVersion: 5` por `protocolVersion: 6`.
- En la tabla de herramientas, añadir tras `hefesto.keyboard`:

```markdown
| `hefesto.media` | `hefesto_media` (`requiresConfirmation: True`) | `{action, count?}` | `{text}` resumen |
```

- Añadir el párrafo:

```markdown
`media` controla volumen, brillo y reproducción con **teclas de sistema**, no con
valores absolutos: `action` es uno de `volume_up`, `volume_down`, `mute`,
`brightness_up`, `brightness_down`, `play_pause`, `next` y `previous`; `count`
(1–10, por defecto 1) solo aplica a volumen y brillo y se rechaza en el resto.
El dispositivo postea la tecla `NX_KEYTYPE_*` correspondiente (subtipo 8,
`NX_KEYDOWN`/`NX_KEYUP`, flag de función) por el tap HID desde el hilo
principal, repitiendo down+up `count` veces, con el mismo permiso de
Accesibilidad y las mismas comprobaciones de entrada segura que el teclado
(`hay un campo con entrada segura en primer plano; no pulso teclas del
sistema`). No lee estado: un «Subido 2 pasos de volumen» confirma la pulsación,
no un valor concreto. Resúmenes estables `Subido/Bajado {n} pasos de volumen`,
`Subido/Bajado {n} paso(s) de brillo` y `Pulsada la tecla de …`, sin estado
`failed`; un reinicio con la pulsación en vuelo la deja `unknown`, sin
reentregarla. La tarjeta es siempre obligatoria y no hay burbuja violeta.
```

- En §5 «Estado, duplicados y fallos»: cambiar el TTL a «(eco, lectura, escritura,
  apps, teclado y multimedia)» y la lista de pendientes con efecto a
  «escrituras, apps, teclado y multimedia».
- En §6, dentro del punto 5 «Apps y multimedia», añadir:

```markdown
   **Corte multimedia (2026-11-01, protocolo v6):** `hefesto_media` con las ocho
   acciones de volumen, brillo y reproducción por teclas de sistema; tarjeta
   siempre, sin lectura de estado, sin subprocess y sin permisos nuevos. El
   detalle y la matriz en vivo viven en §8 y en `docs/implemented.md`.
```

- [ ] **Step 4: Actualizar contratos**

En `docs/contracts.md`, renombrar la sección:

```markdown
## Hefesto v6 — eco, lectura, escritura, apps, teclado y multimedia

Contrato independiente del chat, especificado en [Hefesto §5](./hefesto-plan.md#5-protocolo-v6-eco-lectura-escritura-apps-teclado-y-multimedia).
Usa `protocolVersion: 6`, IDs opacos, campos estrictos, cuerpo máximo 128 KiB y
`output` tipado por herramienta. Nueve herramientas sobre el mismo protocolo:
```

y ajustar el párrafo para incluir la nueva herramienta:

```markdown
`hefesto.media` (`{action, count?}`) controla volumen, brillo y reproducción con
teclas de sistema (acción cerrada; `count` 1–10 solo para volumen y brillo). El
relay expone `hefesto_media` con `requiresConfirmation: True` y
`planConfirmation: True`: cada invocación válida abre una tarjeta y no hay
burbuja.
```

Añadir a las tablas de rutas y herramientas el valor `protocolVersion: 6` y la fila:

```markdown
| `hefesto.media` | `{action, count?}` | `{text}` resumen |
```

- [ ] **Step 5: Verificar que no quedan referencias v5 en la parte vigente**

Run: `rg -n "protocolo v5|protocolVersion: 5|Protocolo v5|Hefesto v5" docs/contracts.md`
Expected: sin resultados.

Run: `sed -n '118,270p' docs/hefesto-plan.md | rg -n "protocolo v5|protocolVersion: 5|Protocolo v5"`
Expected: sin resultados.

No tocar las entradas históricas de §8 del plan (registran los cortes v4/v5 y deben
seguir diciendo lo que decían).

- [ ] **Step 6: Commit**

```bash
git add src/state/orbitPlan.ts src/state/orbitPlan.test.ts docs/hefesto-plan.md docs/contracts.md
git commit -m "docs(hefesto): multimedia en órbitas, contratos y plan v6"
```

---

### Task 7: Verificación global, bundle, despliegue, prueba en vivo y registro

**Files:**
- Modify: `docs/hefesto-plan.md` (§8), `docs/implemented.md`, `docs/implementation-plan.md`

**Interfaces:**
- Consumes: todas las tareas anteriores.
- Produces: `Atlas.app` con v6 instalado, relay v6 desplegado, matriz en vivo verificada y registro documental.

- [ ] **Step 1: Verificación completa**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS y limpio.

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: PASS.

Run: `npm test && npm run build && npm run lint`
Expected: PASS y limpio.

- [ ] **Step 2: Construir el bundle**

```bash
APPLE_SIGNING_IDENTITY="Apple Development: adria_bgcma@hotmail.com (Y329B5UZ34)" npm run tauri build
```
Expected: el paso DMG puede fallar (como siempre); el `.app` queda firmado. Verificar:

```bash
codesign --verify --deep --strict --verbose=2 target/release/bundle/macos/Atlas.app
```

- [ ] **Step 3: Instalar y lanzar con credenciales**

```bash
ditto target/release/bundle/macos/Atlas.app /Applications/Atlas.app
```
```bash
nohup env ATLAS_HEFESTO_CA=$HOME/atlas-hefesto/ca.pem \
  ATLAS_HEFESTO_CERT=$HOME/atlas-hefesto/device.pem \
  ATLAS_HEFESTO_KEY=$HOME/atlas-hefesto/device.key \
  ATLAS_HEFESTO_DEVICE_ID=mac-one \
  /Applications/Atlas.app/Contents/MacOS/atlas-next >/tmp/atlas.log 2>&1 &
```

- [ ] **Step 4: Desplegar el relay (orden: dispositivo primero)**

Run: `./scripts/deploy-relay.sh`
Expected: pull en `atlas-hub`, reinicio del servicio y `/v1/health` OK.

- [ ] **Step 5: Matriz en vivo por HTTP**

Con `POST /v1/send` (`source: "local"`), `GET /v1/events?since=…` y
`POST /v1/confirm` (`allow_once`):

1. «Sube el volumen dos pasos» → tarjeta «Subir el volumen 2 pasos», tras aceptar
   texto «Subido 2 pasos de volumen».
2. «Baja el volumen» → «Bajado 1 paso de volumen».
3. «Silencia el sonido» → «Pulsada la tecla de silencio».
4. «Sube el brillo un paso» → «Subido 1 paso de brillo».
5. «Pausa la música» → «Pulsada la tecla de reproducción/pausa».
6. Validación sin tarjeta: petición directa con `count` 0 / 11 / en `mute` →
   textos exactos y ninguna confirmación en `/v1/events`.
7. Entrada segura: con un diálogo de respuesta oculta en primer plano, volumen →
   «hay un campo con entrada segura en primer plano; no pulso teclas del sistema»
   y sin cambio audible.

Guardar evidencia: el usuario nota volumen/brillo; para mute y play/pausa,
comprobar el indicador del sistema o que la app de música cambia de estado.

- [ ] **Step 6: Auditoría de procesos**

Run: `ps -o pid,comm,rss -ax | rg -i "atlas|textedit" ; pgrep -fl "atlas-next|vite|python.*relay"`
Expected: solo `Atlas.app` (y la app de prueba si sigue abierta); ningún
`vite`, relay local ni proceso huérfano. Cerrar las apps de prueba abiertas por
las pruebas.

- [ ] **Step 7: Registrar**

En `docs/hefesto-plan.md` §8, añadir la entrada fechada con: protocolo v6, piezas
implementadas, textos estables, pruebas ejecutadas (números reales), despliegue y
matriz en vivo. En `docs/implemented.md`, añadir la ficha de multimedia con el
rango de commits de este plan y los resultados reales. En
`docs/implementation-plan.md`, marcar la puerta 5/multimedia como hecha.

- [ ] **Step 8: Commit**

```bash
git add docs/hefesto-plan.md docs/implemented.md docs/implementation-plan.md
git commit -m "docs(hefesto): registro de multimedia (puerta 5)"
```

---

## Self-Review

- **Cobertura del spec:** protocolo v6 (T1), wire/media pura (T2), puente real (T2),
  despacho (T3), journal sin contenido (T4), relay/tarjeta/sin burbuja (T5),
  órbita/contratos/plan (T6), verificación/despliegue/en vivo/registro (T7).
  Todos los textos estables del spec aparecen en el código y en las pruebas.
- **Placeholders:** ninguno; cada paso lleva su código o su comando exacto.
- **Consistencia de tipos:** `MediaAction`, `MediaPayload`, `ToolId::Media`,
  `ToolArguments::Media`, `SystemKey::nx_key_type`, `validate_count`,
  `handle_media` y `KeyboardBridge::press_system` se definen en T2 y se consumen
  con la misma firma en T3–T5.
- **Despliegue:** el dispositivo (v6) se instala antes de desplegar el relay (v6);
  ambos rechazan v5, así que no hay convivencia.
