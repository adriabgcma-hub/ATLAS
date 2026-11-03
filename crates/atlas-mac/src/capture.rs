//! Captura de pantalla con OCR local (puerta 7). La imagen se queda en el
//! Mac; solo el texto reconocido vuelve al modelo.

use crate::wire::{CapturePayload, CaptureTarget, OutputPayload};

const TRUNCATED: &str = "\n… (recortado)";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureFailure {
    Stable(String),
    Detail(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureOutcome {
    pub file_name: String,
    pub text: String,
}

pub trait CaptureBridge: Send + Sync {
    fn capture(&self, target: CaptureTarget)
        -> std::result::Result<CaptureOutcome, CaptureFailure>;
}

fn fit(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut cut = limit.saturating_sub(TRUNCATED.len());
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{}", &text[..cut], TRUNCATED)
}

pub fn handle_capture(
    bridge: &dyn CaptureBridge,
    payload: &CapturePayload,
) -> crate::Result<OutputPayload> {
    match bridge.capture(payload.target) {
        Ok(outcome) => {
            let heading = format!("Captura guardada: {}", outcome.file_name);
            if outcome.text.trim().is_empty() {
                OutputPayload::text(format!("{heading}\nno se reconoció texto"))
            } else {
                let prefix = format!("{heading}\nTexto reconocido:\n");
                let limit = crate::wire::MAX_OUTPUT_BYTES.saturating_sub(prefix.len());
                OutputPayload::text(format!("{prefix}{}", fit(outcome.text.trim(), limit)))
            }
        }
        Err(CaptureFailure::Stable(text)) => OutputPayload::text(text),
        Err(CaptureFailure::Detail(detail)) => {
            OutputPayload::text(format!("no se pudo capturar la pantalla: {detail}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeBridge {
        result: Mutex<std::result::Result<CaptureOutcome, CaptureFailure>>,
        seen: Mutex<Vec<CaptureTarget>>,
    }

    impl FakeBridge {
        fn new(result: std::result::Result<CaptureOutcome, CaptureFailure>) -> Self {
            Self {
                result: Mutex::new(result),
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl CaptureBridge for FakeBridge {
        fn capture(
            &self,
            target: CaptureTarget,
        ) -> std::result::Result<CaptureOutcome, CaptureFailure> {
            self.seen.lock().expect("visto").push(target);
            self.result.lock().expect("resultado").clone()
        }
    }

    fn payload() -> CapturePayload {
        CapturePayload {
            target: CaptureTarget::Screen,
        }
    }

    fn text(output: OutputPayload) -> String {
        output.text_value().expect("texto").to_owned()
    }

    #[test]
    fn la_captura_devuelve_nombre_y_texto() {
        let bridge = FakeBridge::new(Ok(CaptureOutcome {
            file_name: "captura-20261103-120000.png".to_owned(),
            text: "hola mundo\n".to_owned(),
        }));
        let output = handle_capture(&bridge, &payload()).expect("salida");
        assert_eq!(
            text(output),
            "Captura guardada: captura-20261103-120000.png\nTexto reconocido:\nhola mundo"
        );
        assert_eq!(
            *bridge.seen.lock().expect("visto"),
            vec![CaptureTarget::Screen]
        );
    }

    #[test]
    fn sin_texto_lo_dice_sin_inventar() {
        let bridge = FakeBridge::new(Ok(CaptureOutcome {
            file_name: "captura.png".to_owned(),
            text: "   ".to_owned(),
        }));
        let output = handle_capture(&bridge, &payload()).expect("salida");
        assert_eq!(
            text(output),
            "Captura guardada: captura.png\nno se reconoció texto"
        );
    }

    #[test]
    fn un_fallo_estable_se_devuelve_tal_cual() {
        let bridge = FakeBridge::new(Err(CaptureFailure::Stable(
            "falta el permiso de Grabación de pantalla".to_owned(),
        )));
        let output = handle_capture(&bridge, &payload()).expect("salida");
        assert_eq!(text(output), "falta el permiso de Grabación de pantalla");
    }

    #[test]
    fn un_detalle_se_envuelve() {
        let bridge = FakeBridge::new(Err(CaptureFailure::Detail("timeout".to_owned())));
        let output = handle_capture(&bridge, &payload()).expect("salida");
        assert_eq!(text(output), "no se pudo capturar la pantalla: timeout");
    }

    #[test]
    fn el_texto_largo_se_recorta_sin_partir_utf8() {
        let bridge = FakeBridge::new(Ok(CaptureOutcome {
            file_name: "captura.png".to_owned(),
            text: "ñ".repeat(crate::wire::MAX_OUTPUT_BYTES),
        }));
        let output = handle_capture(&bridge, &payload()).expect("salida");
        let value = text(output);
        assert!(value.len() <= crate::wire::MAX_OUTPUT_BYTES);
        assert!(value.ends_with(TRUNCATED));
    }
}
