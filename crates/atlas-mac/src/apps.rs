use crate::wire::{AppRecord, AppsAction, AppsOutput, AppsPayload, OutputPayload};

pub const MAX_APP_LIST: usize = 30;

#[derive(Clone, Debug)]
pub struct AppRef {
    pub name: String,
    pub bundle_id: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseOutcome {
    Gone,
    StillPresent,
}

pub trait AppBridge: Send + Sync {
    fn running(&self) -> Vec<AppRef>;
    fn installed(&self) -> Vec<AppRef>;
    fn open(&self, target: &AppRef) -> Result<(), String>;
    fn terminate(&self, target: &AppRef) -> Result<CloseOutcome, String>;
}

fn norm(value: &str) -> String {
    value.trim().trim_end_matches(".app").to_lowercase()
}

pub fn find_installed<'a>(
    catalog: &'a [AppRef],
    name: Option<&str>,
    bundle: Option<&str>,
) -> Option<&'a AppRef> {
    if let Some(bundle) = bundle {
        if let Some(hit) = catalog.iter().find(|app| app.bundle_id == bundle) {
            return Some(hit);
        }
    }
    let name = name.map(norm);
    catalog.iter().find(|app| Some(norm(&app.name)) == name)
}

pub fn build_list(running: &[AppRef], installed: &[AppRef], query: Option<&str>) -> Vec<AppRecord> {
    let needle = query.map(norm);
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<AppRecord> = Vec::new();
    for app in running {
        if needle
            .as_ref()
            .is_some_and(|q| !norm(&app.name).contains(q))
        {
            continue;
        }
        seen.push(app.bundle_id.clone());
        out.push(AppRecord {
            name: app.name.clone(),
            bundle_id: app.bundle_id.clone(),
            running: true,
        });
    }
    for app in installed {
        if seen.contains(&app.bundle_id) {
            continue;
        }
        if needle
            .as_ref()
            .is_some_and(|q| !norm(&app.name).contains(q))
        {
            continue;
        }
        out.push(AppRecord {
            name: app.name.clone(),
            bundle_id: app.bundle_id.clone(),
            running: false,
        });
    }
    out.truncate(MAX_APP_LIST);
    out
}

pub fn find_running<'a>(
    running: &'a [AppRef],
    name: Option<&str>,
    bundle: Option<&str>,
) -> Option<&'a AppRef> {
    find_installed(running, name, bundle)
}

pub fn handle_apps(bridge: &dyn AppBridge, payload: &AppsPayload) -> crate::Result<OutputPayload> {
    match payload.action {
        AppsAction::List => {
            let apps = build_list(
                &bridge.running(),
                &bridge.installed(),
                payload.query.as_deref(),
            );
            Ok(OutputPayload::Apps(AppsOutput { apps }))
        }
        AppsAction::Open => {
            let catalog = bridge.installed();
            match find_installed(
                &catalog,
                payload.name.as_deref(),
                payload.bundle_id.as_deref(),
            ) {
                None => Ok(OutputPayload::text(format!(
                    "no encuentro «{}» entre las apps instaladas",
                    payload.name.as_deref().unwrap_or("?")
                ))?),
                Some(target) => match bridge.open(target) {
                    Ok(()) => Ok(OutputPayload::text(format!("Abierta «{}»", target.name))?),
                    Err(err) => Ok(OutputPayload::text(format!(
                        "no se pudo abrir «{}»: {err}",
                        target.name
                    ))?),
                },
            }
        }
        AppsAction::Close => {
            let running = bridge.running();
            let installed = bridge.installed();
            let target = find_running(
                &running,
                payload.name.as_deref(),
                payload.bundle_id.as_deref(),
            )
            .or_else(|| {
                find_installed(
                    &installed,
                    payload.name.as_deref(),
                    payload.bundle_id.as_deref(),
                )
            });
            let Some(target) = target else {
                return OutputPayload::text(format!(
                    "«{}» no está abierta",
                    payload.name.as_deref().unwrap_or("?")
                ));
            };
            match bridge.terminate(target) {
                Err(err) => Ok(OutputPayload::text(format!(
                    "no se pudo cerrar «{}»: {err}",
                    target.name
                ))?),
                Ok(CloseOutcome::Gone) => {
                    Ok(OutputPayload::text(format!("Cerrada «{}»", target.name))?)
                }
                Ok(CloseOutcome::StillPresent) => Ok(OutputPayload::text(format!(
                    "«{}» sigue abierta (¿tiene cambios sin guardar?); no la fuerzo",
                    target.name
                ))?),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, id: &str) -> AppRef {
        AppRef {
            name: name.into(),
            bundle_id: id.into(),
            url: format!("file:///Applications/{name}.app"),
        }
    }

    #[test]
    fn resuelve_por_nombre_sin_sufijo_y_ignorando_mayusculas() {
        let catalog = vec![app("Spotify", "com.spotify.client")];
        let hit = find_installed(&catalog, Some("  spotify.app  "), None).unwrap();
        assert_eq!(hit.bundle_id, "com.spotify.client");
    }

    #[test]
    fn resuelve_por_bundle_id_directo() {
        let catalog = vec![app("Mail", "com.apple.mail")];
        assert!(find_installed(&catalog, None, Some("com.apple.mail")).is_some());
    }

    #[test]
    fn nombre_inexistente_devuelve_none() {
        let catalog = vec![app("Mail", "com.apple.mail")];
        assert!(find_installed(&catalog, Some("Gimp"), None).is_none());
    }

    #[test]
    fn lista_prioriza_en_ejecucion_sin_duplicados() {
        let running = vec![app("Spotify", "com.spotify.client")];
        let installed = vec![
            app("Spotify", "com.spotify.client"),
            app("Mail", "com.apple.mail"),
        ];
        let list = build_list(&running, &installed, None);
        assert_eq!(list[0].name, "Spotify");
        assert!(list[0].running);
        assert_eq!(
            list.iter()
                .filter(|a| a.bundle_id == "com.spotify.client")
                .count(),
            1
        );
    }

    #[test]
    fn lista_se_filtra_por_query() {
        let installed = vec![
            app("Mail", "com.apple.mail"),
            app("Safari", "com.apple.safari"),
        ];
        let list = build_list(&[], &installed, Some("saf"));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Safari");
    }

    #[test]
    fn lista_corta_a_treinta() {
        let installed: Vec<AppRef> = (0..40)
            .map(|i| app(&format!("App{i}"), &format!("id{i}")))
            .collect();
        assert_eq!(build_list(&[], &installed, None).len(), MAX_APP_LIST);
    }

    struct FakeBridge {
        running: Vec<AppRef>,
        installed: Vec<AppRef>,
        open_result: Result<(), String>,
        close_result: Result<CloseOutcome, String>,
    }
    impl AppBridge for FakeBridge {
        fn running(&self) -> Vec<AppRef> {
            self.running.clone()
        }
        fn installed(&self) -> Vec<AppRef> {
            self.installed.clone()
        }
        fn open(&self, _: &AppRef) -> Result<(), String> {
            self.open_result.clone()
        }
        fn terminate(&self, _: &AppRef) -> Result<CloseOutcome, String> {
            self.close_result.clone()
        }
    }
    fn payload(action: AppsAction, name: Option<&str>, query: Option<&str>) -> AppsPayload {
        AppsPayload {
            action,
            name: name.map(str::to_owned),
            bundle_id: None,
            query: query.map(str::to_owned),
        }
    }

    #[test]
    fn open_exitoso_da_texto_abierta() {
        let bridge = FakeBridge {
            running: vec![],
            installed: vec![app("Spotify", "com.spotify.client")],
            open_result: Ok(()),
            close_result: Ok(CloseOutcome::Gone),
        };
        let out = handle_apps(&bridge, &payload(AppsAction::Open, Some("Spotify"), None)).unwrap();
        assert_eq!(out.text_value(), Some("Abierta «Spotify»"));
    }

    #[test]
    fn open_sin_encontrar_no_abre() {
        let bridge = FakeBridge {
            running: vec![],
            installed: vec![app("Mail", "com.apple.mail")],
            open_result: Ok(()),
            close_result: Ok(CloseOutcome::Gone),
        };
        let out = handle_apps(&bridge, &payload(AppsAction::Open, Some("Gimp"), None)).unwrap();
        assert!(out.text_value().unwrap().contains("no encuentro"));
    }

    #[test]
    fn close_sigue_abierta_no_fuerza() {
        let bridge = FakeBridge {
            running: vec![app("Mail", "com.apple.mail")],
            installed: vec![],
            open_result: Ok(()),
            close_result: Ok(CloseOutcome::StillPresent),
        };
        let out = handle_apps(&bridge, &payload(AppsAction::Close, Some("Mail"), None)).unwrap();
        assert!(
            out.text_value().unwrap().contains("no la fuerzos")
                || out.text_value().unwrap().contains("sigue abierta")
        );
    }

    #[test]
    fn list_devuelve_apps_no_texto() {
        let bridge = FakeBridge {
            running: vec![app("Spotify", "com.spotify.client")],
            installed: vec![],
            open_result: Ok(()),
            close_result: Ok(CloseOutcome::Gone),
        };
        match handle_apps(&bridge, &payload(AppsAction::List, None, None)).unwrap() {
            OutputPayload::Apps(out) => assert_eq!(out.apps[0].bundle_id, "com.spotify.client"),
            _ => panic!("esperaba OutputPayload::Apps"),
        }
    }

    #[test]
    fn argumentos_apps_ruedan_por_serde_sin_roubar_otras_ramas() {
        let json = r#"{"action":"list","query":"saf"}"#;
        let args: crate::wire::ToolArguments = serde_json::from_str(json).unwrap();
        assert!(matches!(&args, crate::wire::ToolArguments::Apps(payload)
            if payload.action == AppsAction::List
                && payload.query.as_deref() == Some("saf")));
        let back = serde_json::to_string(&args).unwrap();
        let again: crate::wire::ToolArguments = serde_json::from_str(&back).unwrap();
        assert_eq!(args, again);
    }

    #[test]
    fn salida_apps_serializa_en_camelcase_exacta() {
        let out = OutputPayload::Apps(AppsOutput {
            apps: vec![AppRecord {
                name: "Spotify".into(),
                bundle_id: "com.spotify.client".into(),
                running: true,
            }],
        });
        let json = serde_json::to_string(&out).unwrap();
        assert_eq!(
            json,
            r#"{"apps":[{"name":"Spotify","bundleId":"com.spotify.client","running":true}]}"#
        );
        let back: OutputPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(out, back);
    }
}
