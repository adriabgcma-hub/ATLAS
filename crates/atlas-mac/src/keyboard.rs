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
        "a" => 0,
        "s" => 1,
        "d" => 2,
        "f" => 3,
        "h" => 4,
        "g" => 5,
        "z" => 6,
        "x" => 7,
        "c" => 8,
        "v" => 9,
        "b" => 11,
        "q" => 12,
        "w" => 13,
        "e" => 14,
        "r" => 15,
        "y" => 16,
        "t" => 17,
        "1" => 18,
        "2" => 19,
        "3" => 20,
        "4" => 21,
        "6" => 22,
        "5" => 23,
        "9" => 25,
        "7" => 26,
        "8" => 28,
        "0" => 29,
        "o" => 31,
        "u" => 32,
        "i" => 34,
        "p" => 35,
        "l" => 37,
        "j" => 38,
        "k" => 40,
        "n" => 45,
        "m" => 46,
        "return" => 36,
        "tab" => 48,
        "space" => 49,
        "delete" => 51,
        "esc" => 53,
        "left" => 123,
        "right" => 124,
        "down" => 125,
        "up" => 126,
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
                    let unit = if count == 1 {
                        "carácter"
                    } else {
                        "caracteres"
                    };
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
                Ok(()) => OutputPayload::text(format!("Pulsado {combo} en {}", app_label(bridge))),
                Err(err) => OutputPayload::text(format!("no se pudo pulsar: {err}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{KeyboardAction, KeyboardPayload};
    use std::sync::atomic::{AtomicBool, Ordering};

    struct FakeKeyboard {
        granted: bool,
        secure: bool,
        app: Option<String>,
        called: AtomicBool,
        prompted: AtomicBool,
    }

    impl FakeKeyboard {
        fn ok(app: Option<&str>) -> Self {
            Self {
                granted: true,
                secure: false,
                app: app.map(str::to_owned),
                called: AtomicBool::new(false),
                prompted: AtomicBool::new(false),
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
            self.app.clone()
        }

        fn type_text(&self, _: &str) -> Result<(), String> {
            self.called.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn press(&self, _: u8, _: u16) -> Result<(), String> {
            self.called.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn payload(action: KeyboardAction, text: Option<&str>, combo: Option<&str>) -> KeyboardPayload {
        KeyboardPayload {
            action,
            text: text.map(str::to_owned),
            combo: combo.map(str::to_owned),
        }
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
    fn gramatica_cerrada_rechaza_vacios_controles_y_excesos() {
        assert!(parse_combo("").is_err());
        assert!(parse_combo("cmd+shift+alt+ctrl+s").is_err());
        assert!(parse_combo("cmd+zzz").is_err());
        assert!(validate_text("hola\rmundo").is_err());
        assert!(validate_text("hola\0mundo").is_err());
        assert!(validate_text(&"a".repeat(4096)).is_ok());
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
        let out =
            handle_keyboard(&bridge, &payload(KeyboardAction::Type, Some("hola"), None)).unwrap();
        assert_eq!(out.text_value(), Some("Escrito 4 caracteres en «TextEdit»"));
    }

    #[test]
    fn combo_exitoso_resume_atajo_y_app() {
        let bridge = FakeKeyboard::ok(Some("TextEdit"));
        let out = handle_keyboard(
            &bridge,
            &payload(KeyboardAction::Combo, None, Some("cmd+s")),
        )
        .unwrap();
        assert_eq!(out.text_value(), Some("Pulsado cmd+s en «TextEdit»"));
    }

    #[test]
    fn entrada_invalida_no_toca_al_puente() {
        let bridge = FakeKeyboard::ok(Some("TextEdit"));
        let bad_text = handle_keyboard(
            &bridge,
            &payload(KeyboardAction::Type, Some("hola\nmundo"), None),
        )
        .unwrap();
        assert_eq!(
            bad_text.text_value(),
            Some("texto no válido (sin saltos de línea ni controles)")
        );
        let bad_combo =
            handle_keyboard(&bridge, &payload(KeyboardAction::Combo, None, Some("s"))).unwrap();
        assert_eq!(bad_combo.text_value(), Some("atajo no válido"));
        assert!(!bridge.called.load(Ordering::SeqCst));
        assert!(!bridge.prompted.load(Ordering::SeqCst));
    }

    #[test]
    fn entrada_segura_no_teclea() {
        let bridge = FakeKeyboard {
            secure: true,
            ..FakeKeyboard::ok(Some("Safari"))
        };
        let out =
            handle_keyboard(&bridge, &payload(KeyboardAction::Type, Some("hola"), None)).unwrap();
        assert_eq!(out.text_value(), Some(SECURE_INPUT_TEXT));
        assert!(!bridge.called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn sin_permiso_no_teclea_y_pide_una_vez() {
        let bridge = FakeKeyboard {
            granted: false,
            ..FakeKeyboard::ok(Some("TextEdit"))
        };
        let out = handle_keyboard(
            &bridge,
            &payload(KeyboardAction::Combo, None, Some("cmd+s")),
        )
        .unwrap();
        assert_eq!(out.text_value(), Some(NO_PERMISSION_TEXT));
        assert!(bridge.prompted.load(std::sync::atomic::Ordering::SeqCst));
    }
}
