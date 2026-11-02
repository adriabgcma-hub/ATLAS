use crate::keyboard::NO_PERMISSION_TEXT;
use crate::wire::{OutputPayload, WindowAction, WindowRecord, WindowsOutput, WindowsPayload};

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
    fn move_to(&self, target: &WindowTarget, x: i32, y: i32) -> Result<WindowInfo, WindowFailure>;
    fn resize(
        &self,
        target: &WindowTarget,
        width: u32,
        height: u32,
    ) -> Result<WindowInfo, WindowFailure>;
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
        title: payload
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned),
        index: payload.index,
    })
}

fn position(payload: &WindowsPayload) -> Result<(i32, i32), &'static str> {
    match (payload.x, payload.y) {
        (Some(x), Some(y))
            if (-20_000..=20_000).contains(&x) && (-20_000..=20_000).contains(&y) =>
        {
            Ok((x, y))
        }
        _ => Err(BAD_POSITION_TEXT),
    }
}

fn size(payload: &WindowsPayload) -> Result<(u32, u32), &'static str> {
    match (payload.width, payload.height) {
        (Some(width), Some(height))
            if (100..=10_000).contains(&width) && (100..=10_000).contains(&height) =>
        {
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
            Err(WindowFailure::Stable(format!(
                "no encuentro una ventana de «{app}»"
            )))
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
            [] => Err(WindowFailure::Stable(format!(
                "no encuentro una ventana de «{app}»"
            ))),
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

fn failure_output(action: WindowAction, failure: WindowFailure) -> crate::Result<OutputPayload> {
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
            let filter = payload
                .app
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty());
            match bridge.list(filter) {
                Ok(mut windows) => {
                    windows.truncate(MAX_WINDOWS);
                    Ok(OutputPayload::Windows(WindowsOutput {
                        windows: windows.iter().map(WindowInfo::record).collect(),
                    }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Call {
        Permission,
        List(Option<String>),
        Move(WindowTarget, i32, i32),
        Resize(WindowTarget, u32, u32),
        Focus(WindowTarget),
    }

    struct FakeBridge {
        granted: bool,
        windows: Vec<WindowInfo>,
        result: WindowInfo,
        failure: Option<WindowFailure>,
        calls: Mutex<Vec<Call>>,
    }

    impl FakeBridge {
        fn new(windows: Vec<WindowInfo>) -> Self {
            Self {
                granted: true,
                windows,
                result: WindowInfo {
                    x: 10,
                    y: 20,
                    ..window("TextEdit", "Documento", true)
                },
                failure: None,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn denied() -> Self {
            Self {
                granted: false,
                ..Self::new(Vec::new())
            }
        }

        fn failing(failure: WindowFailure) -> Self {
            Self {
                failure: Some(failure),
                ..Self::new(Vec::new())
            }
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }

        fn record(&self, call: Call) {
            self.calls.lock().unwrap().push(call);
        }

        fn outcome(&self) -> Result<WindowInfo, WindowFailure> {
            match &self.failure {
                Some(failure) => Err(failure.clone()),
                None => Ok(self.result.clone()),
            }
        }
    }

    impl WindowBridge for FakeBridge {
        fn permission(&self) -> bool {
            self.record(Call::Permission);
            self.granted
        }

        fn list(&self, app: Option<&str>) -> Result<Vec<WindowInfo>, WindowFailure> {
            self.record(Call::List(app.map(str::to_owned)));
            match &self.failure {
                Some(failure) => Err(failure.clone()),
                None => Ok(self.windows.clone()),
            }
        }

        fn move_to(
            &self,
            target: &WindowTarget,
            x: i32,
            y: i32,
        ) -> Result<WindowInfo, WindowFailure> {
            self.record(Call::Move(target.clone(), x, y));
            self.outcome()
        }

        fn resize(
            &self,
            target: &WindowTarget,
            width: u32,
            height: u32,
        ) -> Result<WindowInfo, WindowFailure> {
            self.record(Call::Resize(target.clone(), width, height));
            self.outcome()
        }

        fn focus(&self, target: &WindowTarget) -> Result<WindowInfo, WindowFailure> {
            self.record(Call::Focus(target.clone()));
            self.outcome()
        }
    }

    fn window(app: &str, title: &str, focused: bool) -> WindowInfo {
        WindowInfo {
            app: app.to_owned(),
            title: title.to_owned(),
            x: 0,
            y: 0,
            width: 800,
            height: 600,
            focused,
            minimized: false,
        }
    }

    fn payload(action: WindowAction) -> WindowsPayload {
        WindowsPayload {
            action,
            app: None,
            title: None,
            index: None,
            x: None,
            y: None,
            width: None,
            height: None,
        }
    }

    fn focus_payload() -> WindowsPayload {
        WindowsPayload {
            app: Some("TextEdit".to_owned()),
            ..payload(WindowAction::FocusWindow)
        }
    }

    fn text(bridge: &FakeBridge, payload: &WindowsPayload) -> String {
        handle_windows(bridge, payload)
            .unwrap()
            .text_value()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn indice_uno_based_elige_la_ventana_y_gana_al_titulo() {
        let windows = vec![
            window("TextEdit", "A", false),
            window("TextEdit", "B", false),
        ];
        assert_eq!(resolve_target("TextEdit", &windows, None, Some(1)), Ok(0));
        assert_eq!(resolve_target("TextEdit", &windows, None, Some(2)), Ok(1));
        assert_eq!(
            resolve_target("TextEdit", &windows, Some("B"), Some(1)),
            Ok(0)
        );
        assert_eq!(
            resolve_target("TextEdit", &windows, None, Some(3)),
            Err(WindowFailure::Stable(
                "no encuentro una ventana de «TextEdit»".to_owned()
            ))
        );
    }

    #[test]
    fn titulo_exacto_gana_a_contiene() {
        let windows = vec![
            window("TextEdit", "Notas de reunión", false),
            window("TextEdit", "Notas", false),
        ];
        assert_eq!(
            resolve_target("TextEdit", &windows, Some("notas"), None),
            Ok(1)
        );
    }

    #[test]
    fn titulo_contenido_cuando_no_hay_exacto() {
        let windows = vec![window("TextEdit", "Documento sin título", false)];
        assert_eq!(
            resolve_target("TextEdit", &windows, Some("TÍTULO"), None),
            Ok(0)
        );
    }

    #[test]
    fn titulo_ambiguo_pide_concretar() {
        let windows = vec![
            window("TextEdit", "Documento A", false),
            window("TextEdit", "Documento B", false),
        ];
        assert_eq!(
            resolve_target("TextEdit", &windows, Some("documento"), None),
            Err(WindowFailure::Stable(
                "hay 2 ventanas de «TextEdit» que coinciden; concreta el título o el índice"
                    .to_owned()
            ))
        );
    }

    #[test]
    fn titulo_sin_coincidencia_es_estable() {
        let windows = vec![window("TextEdit", "Documento", false)];
        assert_eq!(
            resolve_target("TextEdit", &windows, Some("otra"), None),
            Err(WindowFailure::Stable(
                "no encuentro una ventana de «TextEdit»".to_owned()
            ))
        );
    }

    #[test]
    fn sin_titulo_ni_indice_usa_foco_y_luego_la_primera() {
        let focused = vec![
            window("TextEdit", "A", false),
            window("TextEdit", "B", true),
        ];
        assert_eq!(resolve_target("TextEdit", &focused, None, None), Ok(1));
        let none_focused = vec![
            window("TextEdit", "A", false),
            window("TextEdit", "B", false),
        ];
        assert_eq!(resolve_target("TextEdit", &none_focused, None, None), Ok(0));
        assert_eq!(
            resolve_target("TextEdit", &[], None, None),
            Err(WindowFailure::Stable(
                "no encuentro una ventana de «TextEdit»".to_owned()
            ))
        );
    }

    #[test]
    fn objetivo_exige_app_y_limita_el_indice() {
        assert_eq!(
            build_target(&payload(WindowAction::MoveWindow)),
            Err(MISSING_APP_TEXT)
        );
        assert_eq!(
            build_target(&WindowsPayload {
                app: Some("   ".to_owned()),
                ..payload(WindowAction::MoveWindow)
            }),
            Err(MISSING_APP_TEXT)
        );
        assert_eq!(
            build_target(&WindowsPayload {
                app: Some("TextEdit".to_owned()),
                index: Some(0),
                ..payload(WindowAction::MoveWindow)
            }),
            Err(BAD_INDEX_TEXT)
        );
        assert_eq!(
            build_target(&WindowsPayload {
                app: Some("TextEdit".to_owned()),
                index: Some(100),
                ..payload(WindowAction::MoveWindow)
            }),
            Err(BAD_INDEX_TEXT)
        );
        let target = build_target(&WindowsPayload {
            app: Some("  TextEdit ".to_owned()),
            title: Some("  Documento  ".to_owned()),
            index: Some(99),
            ..payload(WindowAction::FocusWindow)
        })
        .unwrap();
        assert_eq!(
            target,
            WindowTarget {
                app: "TextEdit".to_owned(),
                title: Some("Documento".to_owned()),
                index: Some(99),
            }
        );
        let without_title = build_target(&WindowsPayload {
            app: Some("TextEdit".to_owned()),
            title: Some("   ".to_owned()),
            ..payload(WindowAction::FocusWindow)
        })
        .unwrap();
        assert_eq!(without_title.title, None);
    }

    #[test]
    fn posicion_y_tamano_tienen_rango() {
        let base = payload(WindowAction::MoveWindow);
        assert_eq!(
            position(&WindowsPayload {
                x: Some(0),
                y: Some(0),
                ..base.clone()
            }),
            Ok((0, 0))
        );
        assert_eq!(
            position(&WindowsPayload {
                x: Some(-20_000),
                y: Some(20_000),
                ..base.clone()
            }),
            Ok((-20_000, 20_000))
        );
        assert_eq!(
            position(&WindowsPayload {
                x: Some(-20_001),
                y: Some(0),
                ..base.clone()
            }),
            Err(BAD_POSITION_TEXT)
        );
        assert_eq!(
            position(&WindowsPayload {
                x: Some(20_001),
                y: Some(0),
                ..base.clone()
            }),
            Err(BAD_POSITION_TEXT)
        );
        assert_eq!(
            position(&WindowsPayload {
                x: Some(0),
                ..base.clone()
            }),
            Err(BAD_POSITION_TEXT)
        );
        assert_eq!(position(&base), Err(BAD_POSITION_TEXT));
        assert_eq!(
            size(&WindowsPayload {
                width: Some(100),
                height: Some(10_000),
                ..base.clone()
            }),
            Ok((100, 10_000))
        );
        assert_eq!(
            size(&WindowsPayload {
                width: Some(99),
                height: Some(100),
                ..base.clone()
            }),
            Err(BAD_SIZE_TEXT)
        );
        assert_eq!(
            size(&WindowsPayload {
                width: Some(100),
                height: Some(10_001),
                ..base.clone()
            }),
            Err(BAD_SIZE_TEXT)
        );
        assert_eq!(
            size(&WindowsPayload {
                width: Some(800),
                ..base
            }),
            Err(BAD_SIZE_TEXT)
        );
    }

    #[test]
    fn list_sin_permiso_no_toca_al_puente() {
        let bridge = FakeBridge::denied();
        assert_eq!(
            text(&bridge, &payload(WindowAction::ListWindows)),
            NO_PERMISSION_TEXT
        );
        assert_eq!(bridge.calls(), vec![Call::Permission]);
    }

    #[test]
    fn list_con_permiso_devuelve_windows_truncado_a_30() {
        let windows = (0..35)
            .map(|i| window("TextEdit", &format!("V{i}"), i == 0))
            .collect();
        let bridge = FakeBridge::new(windows);
        let output = handle_windows(&bridge, &payload(WindowAction::ListWindows)).unwrap();
        assert!(output.text_value().is_none());
        let OutputPayload::Windows(out) = output else {
            panic!("la lista debe producir OutputPayload::Windows");
        };
        assert_eq!(out.windows.len(), MAX_WINDOWS);
        assert_eq!(
            out.windows[0],
            WindowRecord {
                app: "TextEdit".to_owned(),
                title: "V0".to_owned(),
                x: 0,
                y: 0,
                width: 800,
                height: 600,
                focused: true,
                minimized: false,
            }
        );
        assert_eq!(bridge.calls(), vec![Call::Permission, Call::List(None)]);
    }

    #[test]
    fn list_filtra_por_app_y_la_vacia_equivale_a_sin_filtro() {
        let bridge = FakeBridge::new(Vec::new());
        handle_windows(
            &bridge,
            &WindowsPayload {
                app: Some("  TextEdit ".to_owned()),
                ..payload(WindowAction::ListWindows)
            },
        )
        .unwrap();
        handle_windows(
            &bridge,
            &WindowsPayload {
                app: Some("   ".to_owned()),
                ..payload(WindowAction::ListWindows)
            },
        )
        .unwrap();
        assert_eq!(
            bridge.calls(),
            vec![
                Call::Permission,
                Call::List(Some("TextEdit".to_owned())),
                Call::Permission,
                Call::List(None),
            ]
        );
    }

    #[test]
    fn move_resize_y_focus_resumen_con_textos_exactos() {
        let bridge = FakeBridge::new(Vec::new());
        let move_payload = WindowsPayload {
            app: Some("TextEdit".to_owned()),
            x: Some(300),
            y: Some(400),
            ..payload(WindowAction::MoveWindow)
        };
        assert_eq!(
            text(&bridge, &move_payload),
            "Ventana «Documento» de «TextEdit» movida a 10,20"
        );
        let resize_payload = WindowsPayload {
            app: Some("TextEdit".to_owned()),
            width: Some(1200),
            height: Some(800),
            ..payload(WindowAction::ResizeWindow)
        };
        assert_eq!(
            text(&bridge, &resize_payload),
            "Ventana «Documento» de «TextEdit» redimensionada a 800×600"
        );
        assert_eq!(
            text(&bridge, &focus_payload()),
            "Ventana «Documento» de «TextEdit» traída al frente"
        );
        let target = WindowTarget {
            app: "TextEdit".to_owned(),
            title: None,
            index: None,
        };
        assert_eq!(
            bridge.calls(),
            vec![
                Call::Permission,
                Call::Move(target.clone(), 300, 400),
                Call::Permission,
                Call::Resize(target.clone(), 1200, 800),
                Call::Permission,
                Call::Focus(target),
            ]
        );
    }

    #[test]
    fn validacion_antes_que_permiso_y_puente() {
        let bridge = FakeBridge::denied();
        assert_eq!(
            text(&bridge, &payload(WindowAction::MoveWindow)),
            MISSING_APP_TEXT
        );
        assert_eq!(
            text(
                &bridge,
                &WindowsPayload {
                    app: Some("TextEdit".to_owned()),
                    ..payload(WindowAction::MoveWindow)
                }
            ),
            BAD_POSITION_TEXT
        );
        assert_eq!(
            text(
                &bridge,
                &WindowsPayload {
                    app: Some("TextEdit".to_owned()),
                    ..payload(WindowAction::ResizeWindow)
                }
            ),
            BAD_SIZE_TEXT
        );
        assert_eq!(
            text(
                &bridge,
                &WindowsPayload {
                    app: Some("TextEdit".to_owned()),
                    index: Some(0),
                    ..payload(WindowAction::FocusWindow)
                }
            ),
            BAD_INDEX_TEXT
        );
        assert!(bridge.calls().is_empty());
        assert_eq!(text(&bridge, &focus_payload()), NO_PERMISSION_TEXT);
        assert_eq!(bridge.calls(), vec![Call::Permission]);
    }

    #[test]
    fn fallo_estable_pasa_tal_cual_y_el_detalle_se_envuelve() {
        let stable = FakeBridge::failing(WindowFailure::Stable("«X» no está abierta".to_owned()));
        assert_eq!(text(&stable, &focus_payload()), "«X» no está abierta");
        assert_eq!(
            text(&stable, &payload(WindowAction::ListWindows)),
            "«X» no está abierta"
        );

        for (action, expected) in [
            (
                WindowAction::ListWindows,
                "no se pudo listar la ventana: sin puente",
            ),
            (
                WindowAction::MoveWindow,
                "no se pudo mover la ventana: sin puente",
            ),
            (
                WindowAction::ResizeWindow,
                "no se pudo redimensionar la ventana: sin puente",
            ),
            (
                WindowAction::FocusWindow,
                "no se pudo traer la ventana: sin puente",
            ),
        ] {
            let bridge = FakeBridge::failing(WindowFailure::Detail("sin puente".to_owned()));
            let mut request = payload(action);
            request.app = Some("TextEdit".to_owned());
            if action == WindowAction::MoveWindow {
                request.x = Some(1);
                request.y = Some(2);
            }
            if action == WindowAction::ResizeWindow {
                request.width = Some(800);
                request.height = Some(600);
            }
            assert_eq!(text(&bridge, &request), expected);
        }
    }
}
