//! Consultas del sistema de Hefesto (puerta 6, corte 1): un conjunto cerrado de
//! adaptadores de solo lectura. La selección de binario y argumentos vive en el
//! dispositivo; el modelo solo elige la acción.

use crate::wire::{OutputPayload, SystemAction, SystemPayload};

pub const EMPTY_OUTPUT_TEXT: &str = "la consulta no devolvió datos";

/// Fallo del adaptador: mensaje final (`Stable`) o detalle que se envuelve con
/// «no se pudo consultar el sistema: …» (`Detail`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SystemFailure {
    Stable(String),
    Detail(String),
}

pub trait SystemBridge: Send + Sync {
    fn run(&self, action: SystemAction) -> std::result::Result<String, SystemFailure>;
}

pub fn heading(action: SystemAction) -> &'static str {
    match action {
        SystemAction::Os => "Sistema",
        SystemAction::Disk => "Disco",
        SystemAction::Uptime => "Tiempo encendido",
        SystemAction::Memory => "Memoria",
    }
}

pub fn handle_system(
    bridge: &dyn SystemBridge,
    payload: &SystemPayload,
) -> crate::Result<OutputPayload> {
    match bridge.run(payload.action) {
        Ok(text) => {
            let text = text.trim();
            if text.is_empty() {
                OutputPayload::text(EMPTY_OUTPUT_TEXT)
            } else {
                OutputPayload::text(format!("{}:\n{text}", heading(payload.action)))
            }
        }
        Err(SystemFailure::Stable(text)) => OutputPayload::text(text),
        Err(SystemFailure::Detail(detail)) => {
            OutputPayload::text(format!("no se pudo consultar el sistema: {detail}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeBridge {
        result: Mutex<std::result::Result<String, SystemFailure>>,
        seen: Mutex<Vec<SystemAction>>,
    }

    impl FakeBridge {
        fn new(result: std::result::Result<String, SystemFailure>) -> Self {
            Self {
                result: Mutex::new(result),
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl SystemBridge for FakeBridge {
        fn run(&self, action: SystemAction) -> std::result::Result<String, SystemFailure> {
            self.seen.lock().expect("visto").push(action);
            self.result.lock().expect("resultado").clone()
        }
    }

    fn payload(action: SystemAction) -> SystemPayload {
        SystemPayload { action }
    }

    fn text(output: OutputPayload) -> String {
        output.text_value().expect("texto").to_owned()
    }

    #[test]
    fn cada_consulta_se_adapta_y_lleva_encabezado() {
        let cases = [
            (SystemAction::Os, "ProductVersion: 15.0", "Sistema"),
            (SystemAction::Disk, "/ 100G 50G 50G", "Disco"),
            (SystemAction::Uptime, "up 3 days", "Tiempo encendido"),
            (SystemAction::Memory, "Pages free: 1000", "Memoria"),
        ];
        for (action, raw, heading) in cases {
            let bridge = FakeBridge::new(Ok(raw.to_owned()));
            let output = handle_system(&bridge, &payload(action)).expect("salida");
            assert_eq!(text(output), format!("{heading}:\n{raw}"));
            assert_eq!(*bridge.seen.lock().expect("visto"), vec![action]);
        }
    }

    #[test]
    fn la_salida_se_recorta_de_espacios() {
        let bridge = FakeBridge::new(Ok("  datos\n".to_owned()));
        let output = handle_system(&bridge, &payload(SystemAction::Os)).expect("salida");
        assert_eq!(text(output), "Sistema:\ndatos");
    }

    #[test]
    fn una_salida_vacia_es_texto_estable() {
        let bridge = FakeBridge::new(Ok("   ".to_owned()));
        let output = handle_system(&bridge, &payload(SystemAction::Uptime)).expect("salida");
        assert_eq!(text(output), EMPTY_OUTPUT_TEXT);
    }

    #[test]
    fn un_fallo_estable_se_devuelve_tal_cual() {
        let bridge = FakeBridge::new(Err(SystemFailure::Stable("sin datos".to_owned())));
        let output = handle_system(&bridge, &payload(SystemAction::Disk)).expect("salida");
        assert_eq!(text(output), "sin datos");
    }

    #[test]
    fn un_detalle_se_envuelve_con_la_accion() {
        let bridge = FakeBridge::new(Err(SystemFailure::Detail(
            "la consulta no respondió a tiempo".to_owned(),
        )));
        let output = handle_system(&bridge, &payload(SystemAction::Memory)).expect("salida");
        assert_eq!(
            text(output),
            "no se pudo consultar el sistema: la consulta no respondió a tiempo"
        );
    }
}
