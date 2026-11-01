//! Puente real de teclado de Hefesto (Task 4): `CGEvent` + permiso TCC.
//!
//! CGEvent puede postear desde cualquier hilo, pero el diálogo de permiso es
//! UI del sistema y AppKit (`NSWorkspace`) exige el hilo principal. Todo
//! acceso se reenvía con `run_on_main_thread`: el hilo de Hefesto espera
//! fuera del hilo principal, con timeout y sin dormir dentro del cierre.
//!
//! Este módulo solo existe en macOS: CGEvent y AppKit no están disponibles en
//! el resto de plataformas.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use atlas_mac::keyboard::{KeyboardBridge, MOD_ALT, MOD_CMD, MOD_CTRL, MOD_SHIFT};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use objc2_app_kit::NSWorkspace;
use tauri::AppHandle;

/// Viaje máximo al hilo principal para una consulta o un posteo acotado.
const MAIN_ROUNDTRIP: Duration = Duration::from_secs(5);
/// CGEvent escribe como mucho 20 unidades UTF-16 por evento Unicode.
const CHUNK_UTF16: usize = 20;
/// Errores de canal estables, cortos (los consume `handle_keyboard`).
const FORWARD_ERROR: &str = "no se pudo reenviar al hilo principal";
const TIMEOUT_ERROR: &str = "timeout del hilo principal";

// Justificación: `core-graphics` no expone el preflight/request de TCC para
// postear eventos; son símbolos públicos de CoreGraphics y el bloque solo
// declara sus dos firmas, sin punteros ni estado propio.
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
}

// Justificación: el símbolo público vive en HIToolbox, subframework que no se
// enlaza por separado; el paraguas Carbon exporta la misma función.
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}

/// Permiso de posteo vigente, sin abrir diálogos.
fn preflight_post_access() -> bool {
    // Justificación: función C pública sin argumentos; solo consulta el TCC.
    unsafe { CGPreflightPostEventAccess() }
}

/// Pide el permiso de posteo: macOS decide si muestra el diálogo.
fn request_post_access() -> bool {
    // Justificación: función C pública sin argumentos; solo consulta el TCC.
    unsafe { CGRequestPostEventAccess() }
}

/// Entrada segura activa (campos de contraseña y diálogos del sistema).
fn secure_event_input() -> bool {
    // Justificación: función C pública sin argumentos; solo lee un flag del sistema.
    unsafe { IsSecureEventInputEnabled() }
}

/// Trocea el texto en unidades UTF-16 sin partir nunca un par sustituto.
fn utf16_chunks(text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut units = 0;
    for (index, character) in text.char_indices() {
        let width = character.len_utf16();
        if units + width > CHUNK_UTF16 {
            chunks.push(&text[start..index]);
            start = index;
            units = 0;
        }
        units += width;
    }
    if start < text.len() {
        chunks.push(&text[start..]);
    }
    chunks
}

/// Traduce los bits de `keyboard.rs` a flags de CGEvent.
fn cg_flags(modifiers: u8) -> CGEventFlags {
    let mut flags = CGEventFlags::empty();
    if modifiers & MOD_CMD != 0 {
        flags |= CGEventFlags::CGEventFlagCommand;
    }
    if modifiers & MOD_SHIFT != 0 {
        flags |= CGEventFlags::CGEventFlagShift;
    }
    if modifiers & MOD_ALT != 0 {
        flags |= CGEventFlags::CGEventFlagAlternate;
    }
    if modifiers & MOD_CTRL != 0 {
        flags |= CGEventFlags::CGEventFlagControl;
    }
    flags
}

/// Fuente de eventos del sistema HID: la misma que usa el teclado físico.
fn event_source() -> Result<CGEventSource, String> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|()| "no se pudo crear la fuente de eventos".to_string())
}

/// Postea el texto como eventos Unicode (keycode 0), down y up por trozo.
fn post_text_on_main(text: &str) -> Result<(), String> {
    let source = event_source()?;
    for chunk in utf16_chunks(text) {
        for key_down in [true, false] {
            let event = CGEvent::new_keyboard_event(source.clone(), 0, key_down)
                .map_err(|()| "no se pudo crear el evento de teclado".to_string())?;
            event.set_string(chunk);
            event.post(CGEventTapLocation::HID);
        }
    }
    Ok(())
}

/// Postea un atajo con los flags dados (down y up, mismo keycode/estado).
fn post_combo_on_main(flags: CGEventFlags, keycode: u16) -> Result<(), String> {
    let source = event_source()?;
    for key_down in [true, false] {
        let event = CGEvent::new_keyboard_event(source.clone(), keycode, key_down)
            .map_err(|()| "no se pudo crear el evento de teclado".to_string())?;
        event.set_flags(flags);
        event.post(CGEventTapLocation::HID);
    }
    Ok(())
}

pub struct CgKeyboard {
    handle: AppHandle,
    prompted: AtomicBool,
}

impl CgKeyboard {
    pub fn new(handle: AppHandle) -> Self {
        Self {
            handle,
            prompted: AtomicBool::new(false),
        }
    }

    /// Viaje corto al hilo principal: el trabajo (CG/AppKit) se ejecuta allí y
    /// el resultado vuelve por canal. Quien espera es el hilo llamante.
    fn on_main<T, F>(&self, work: F) -> Result<T, String>
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
            .map_err(|_| FORWARD_ERROR.to_string())?;
        rx.recv_timeout(MAIN_ROUNDTRIP)
            .map_err(|_| TIMEOUT_ERROR.to_string())
    }
}

impl KeyboardBridge for CgKeyboard {
    fn can_post(&self, prompt: bool) -> bool {
        if self.on_main(preflight_post_access).unwrap_or(false) {
            return true;
        }
        // El diálogo se pide como mucho UNA vez por proceso: el primer hilo
        // que gana el `swap` cruza a pedirlo; los demás no vuelven a abrirlo.
        if !prompt || self.prompted.swap(true, Ordering::SeqCst) {
            return false;
        }
        let _ = self.on_main(request_post_access);
        // Sin permiso concedido no se postea nada: el resultado decide.
        self.on_main(preflight_post_access).unwrap_or(false)
    }

    fn secure_input(&self) -> bool {
        // Conservador: si el hilo principal no responde, se asume entrada
        // segura y no se teclea.
        self.on_main(secure_event_input).unwrap_or(true)
    }

    fn frontmost_app(&self) -> Option<String> {
        self.on_main(|| {
            NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .and_then(|app| app.localizedName())
                .map(|name| name.to_string())
        })
        .ok()
        .flatten()
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        let text = text.to_owned();
        self.on_main(move || post_text_on_main(&text))?
    }

    fn press(&self, modifiers: u8, keycode: u16) -> Result<(), String> {
        let flags = cg_flags(modifiers);
        self.on_main(move || post_combo_on_main(flags, keycode))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trocea_por_unidades_utf16_sin_partir_pares_sustitutos() {
        let ascii = "a".repeat(45);
        let chunks = utf16_chunks(&ascii);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|chunk| chunk.len() <= CHUNK_UTF16));
        assert_eq!(chunks.concat(), ascii);
    }

    #[test]
    fn un_emoji_de_dos_unidades_nunca_se_parte() {
        let emojis = "😀".repeat(11); // 22 unidades UTF-16
        let chunks = utf16_chunks(&emojis);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].encode_utf16().count(), 20);
        assert_eq!(chunks[1].encode_utf16().count(), 2);
        assert_eq!(chunks.concat(), emojis);
    }

    #[test]
    fn texto_vacio_no_produce_trozos() {
        assert!(utf16_chunks("").is_empty());
    }

    #[test]
    fn los_modificadores_se_traducen_a_flags() {
        assert_eq!(cg_flags(0), CGEventFlags::empty());
        assert_eq!(
            cg_flags(MOD_CMD | MOD_SHIFT),
            CGEventFlags::CGEventFlagCommand | CGEventFlags::CGEventFlagShift
        );
        assert_eq!(
            cg_flags(MOD_ALT | MOD_CTRL),
            CGEventFlags::CGEventFlagAlternate | CGEventFlags::CGEventFlagControl
        );
    }
}
