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
        assert_eq!(
            validate_count(MediaAction::BrightnessDown, Some(10)),
            Ok(10)
        );
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
