# Control de apps de Hefesto (puerta 7, corte 1) — Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Que Hefesto pueda abrir apps instaladas, cerrarlas de forma elegante y listar disponibles/en ejecución desde el chat, con una sola herramienta de acción cerrada y sin abrir acceso a shell ni a API privada de macOS.

**Architecture:** El dispositivo (Rust, `crates/atlas-mac`) enruta un nuevo `toolId` `hefesto.apps` a un módulo `apps.rs` de lógica pura que resuelve nombre→app y construye la lista acotada; las llamadas reales a AppKit (`NSWorkspace`) viven detrás de un trait inyectado `AppBridge` que implementa `src-tauri` en el hilo principal. El relay (Python) expone **una** herramienta `hefesto_apps` (`requiresConfirmation: False`), delega como las lecturas y emite la burbuja violeta HEFESTO con `_hefesto_note()` tras un `open`/`close` correcto. El journal trata `hefesto.apps` como operación sin contenido sensible y una invocación en vuelo queda `unknown` tras reinicio, sin reejecutar.

**Tech Stack:** Rust (Tauri v2, `objc2` + `objc2-app-kit`, `plist`), Python 3 (relay + journal `sqlite3`), TypeScript/React + vitest.

**Spec:** [docs/superpowers/specs/2026-10-31-hefesto-apps-design.md](file:///Users/adria/Developer/project-1/atlas-next/docs/superpowers/specs/2026-10-31-hefesto-apps-design.md)

## Global Constraints

- `protocolVersion` sube de **3 a 4** en los dos extremos (wire Rust y Python) y en todos los fixtures/tests; ambos lados deben rechazar versiones distintas.
- `toolId` del dispositivo: **`hefesto.apps`**. Nombre de herramienta del relay expuesta al modelo: **`hefesto_apps`**.
- **Una sola** herramienta con `action ∈ {open, close, list}` (no tres), para minimizar tokens del catálogo.
- `requiresConfirmation: False` (trivial por adaptador); nunca abre tarjeta genérica.
- `open`/`close` con éxito → **una** burbuja violeta `agent: hefesto` vía `_hefesto_note()`. `list` → resultado síncrono al modelo, **sin** burbuja.
- Cierre **solo suave** (`terminate`); nunca `forceTerminate`, nunca descartar cambios, nunca aceptar diálogos de guardado.
- `list` prioriza en‑ejecución + coincidencias de `query` y corta a **≤30** entradas.
- **Sin API privada** (`_LSCopyAllApplicationURLs`, `LSApplicationWorkspace`) y **sin lista de binarios**: solo `NSWorkspace`/`runningApplications`/`openApplicationAtURL`/`terminate` + escaneo público de `Info.plist` en `/Applications`, `/System/Applications` y `~/Applications`.
- El reenvío al hilo principal de AppKit se hace vía el trait `AppBridge` inyectado (atlas-mac **no** conoce el `AppHandle`); toda llamada lleva timeout.
- Journal: `hefesto.apps` no persiste contenido sensible (solo toolId/estado/ids, como `file_info`); los argumentos son identificadores de app (no documento de usuario).
- Regla del Mac: sin procesos nuevos que queden vivos ni RAM añadida al cerrar la prueba.
- Idioma del registro: commits en español con alcance `feat(hefesto)` / `fix(hefesto)` / `docs(hefesto)`.

---

### Task 1: Bump de protocolo v3 → v4

**Files:**
- Modify: `crates/atlas-mac/src/wire.rs:51-68` (`ProtocolVersion::try_from` y `From<ProtocolVersion> for u64`)
- Modify: literales `protocolVersion == 3` / `br#"{"protocolVersion":3...}"#` en `crates/atlas-mac/src/client.rs:297,303`, `crates/atlas-mac/src/runtime.rs` (tests), `crates/atlas-mac/src/executor.rs` (tests)
- Modify: `server/hefesto.py` (docstring línea 1 y todos los literales `protocolVersion ... 3`)
- Modify: `server/atlas_relay.py` (los 5 literales `protocolVersion 3` de submits/outcome/errors)
- Modify: `server/test_hefesto_delegation.py` y `server/test_hefesto.py` (literales `3`)

**Interfaces:**
- Consumes: nada.
- Produces: `ProtocolVersion` que serializa a `4` y acepta solo `4`; el journal y el relay validan `protocolVersion == 4`. Todas las tareas siguientes asumen 4.

- [ ] **Step 1: Localizar todos los literales de versión**

Run: `cd /Users/adria/Developer/project-1/atlas-next && rg -n 'protocolVersion' crates server | rg -n '3|4'`
Deja a la vista cada `3` relacionado con versión (no el `3` de timeouts ni `TURN`/reintentos).

- [ ] **Step 2: Escribir el test fallido de versión (Rust)**

En `crates/atlas-mac/src/wire.rs`, dentro de su `#[cfg(test)] mod tests`, añade:

```rust
#[test]
fn protocol_version_is_four() {
    assert_eq!(u64::from(ProtocolVersion), 4);
    assert!(ProtocolVersion::try_from(4u64).is_ok());
    assert!(ProtocolVersion::try_from(3u64).is_err());
}
```

- [ ] **Step 3: Ejecutar para ver que falla**

Run: `cargo test -p atlas-mac --lib protocol_version_is_four`
Expected: FAIL (`u64::from` devuelve 3; `try_from(4)` da error).

- [ ] **Step 4: Implementar el bump en `wire.rs`**

Cambia en `ProtocolVersion::try_from` el `if value == 3` por `if value == 4`, y en `impl From<ProtocolVersion> for u64` el `3` por `4`.

- [ ] **Step 5: Volcar los literales `3` de versión en tests Rust**

En `client.rs`, `runtime.rs` y `executor.rs`, sustituye en los fixtures/brackets solo los `protocolVersion` que valen 3 por 4 (los de texto crudo `br#"{"protocolVersion":3...}"#` y los JSON esperados). No toques `3` que sean timeouts/turnos.

- [ ] **Step 6: Volcar los literales en Python**

En `server/hefesto.py`: la docstring de la línea 1 («Hefesto v3 …» → «Hefesto v4 …») y cada `{"protocolVersion": 3, ...}` / `body["protocolVersion"] != 3` → `4`. En `server/atlas_relay.py`: los payloads de submit/outcome/error que ponen `3` → `4`. En los dos ficheros de test, los `3` de versión → `4`.

Run: `cd server && rg -n 'protocolVersion.{0,4}3' *.py`
Expected tras el cambio: sin coincidencias de versión (solo quedarían timeouts/otras).

- [ ] **Step 7: Ejecutar suites**

Run: `cargo test -p atlas-mac`
Run: `cd server && python3 -m unittest discover -p "test_*.py"`
Expected: PASS en ambas (nada más cambió de comportamiento).

- [ ] **Step 8: Commit**

```bash
git add crates server
git commit -m "feat(hefesto): protocolo v4 de preparación para control de apps"
```

---

### Task 2: Módulo `apps.rs` (lógica pura + trait `AppBridge`)

**Files:**
- Create: `crates/atlas-mac/src/apps.rs`
- Modify: `crates/atlas-mac/src/wire.rs` (nuevos `AppsAction`, `AppsPayload`, `AppRecord`, `AppsOutput`; rama `ToolArguments::Apps`; rama `OutputPayload::Apps`; `ToolId::Apps` → `hefesto.apps`)
- Modify: `crates/atlas-mac/src/lib.rs:3-13` (`pub mod apps;` y `pub use apps::{AppBridge, AppRef, CloseOutcome};`)

**Interfaces:**
- Consumes: `ProtocolVersion` v4 (Task 1).
- Produces:
  - `pub enum AppsAction { Open, Close, List }` (serde lowercase).
  - `pub struct AppsPayload { action: AppsAction, name: Option<String>, bundle_id: Option<String>, query: Option<String> }` (serde camelCase: `name`/`bundleId`/`query`, `deny_unknown_fields`).
  - `pub struct AppRef { pub name: String, pub bundle_id: String, pub url: String }`.
  - `pub enum CloseOutcome { Gone, StillPresent }`.
  - `pub trait AppBridge: Send + Sync { fn running(&self) -> Vec<AppRef>; fn installed(&self) -> Vec<AppRef>; fn open(&self, target: &AppRef) -> Result<(), String>; fn terminate(&self, target: &AppRef) -> Result<CloseOutcome, String>; }`.
  - `pub fn handle_apps(bridge: &dyn AppBridge, payload: &AppsPayload) -> crate::Result<OutputPayload>` (propaga con `?`, sin `unwrap`; corregido en el fix round de T2).
  - Funciones puras auxiliares: `find_installed`, `find_running`, `build_list`, `norm`.

- [ ] **Step 1: Escribir los tipos wire**

En `wire.rs`, junto a los payloads de escritura, añade:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppsAction { Open, Close, List }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppsPayload {
    pub action: AppsAction,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppRecord {
    pub name: String,
    pub bundle_id: String,
    pub running: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppsOutput {
    pub apps: Vec<AppRecord>,
}
```

Añade `Apps(AppsPayload)` a `ToolArguments`, `Apps(AppsOutput)` a `OutputPayload`, y la rama `#[serde(rename = "hefesto.apps")] Apps` a `ToolId`.

- [ ] **Step 2: Escribir el test fallido del trait y la lógica pura**

Crea `crates/atlas-mac/src/apps.rs` con la firma y un test que usa un `FakeBridge`:

```rust
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

pub fn find_installed<'a>(catalog: &'a [AppRef], name: Option<&str>, bundle: Option<&str>) -> Option<&'a AppRef> {
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
        if needle.as_ref().is_some_and(|q| !norm(&app.name).contains(q)) {
            continue;
        }
        seen.push(app.bundle_id.clone());
        out.push(AppRecord { name: app.name.clone(), bundle_id: app.bundle_id.clone(), running: true });
    }
    for app in installed {
        if seen.contains(&app.bundle_id) {
            continue;
        }
        if needle.as_ref().is_some_and(|q| !norm(&app.name).contains(q)) {
            continue;
        }
        out.push(AppRecord { name: app.name.clone(), bundle_id: app.bundle_id.clone(), running: false });
    }
    out.truncate(MAX_APP_LIST);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, id: &str) -> AppRef {
        AppRef { name: name.into(), bundle_id: id.into(), url: format!("file:///Applications/{name}.app") }
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
        let installed = vec![app("Spotify", "com.spotify.client"), app("Mail", "com.apple.mail")];
        let list = build_list(&running, &installed, None);
        assert_eq!(list[0].name, "Spotify");
        assert!(list[0].running);
        assert_eq!(list.iter().filter(|a| a.bundle_id == "com.spotify.client").count(), 1);
    }

    #[test]
    fn lista_se_filtra_por_query() {
        let installed = vec![app("Mail", "com.apple.mail"), app("Safari", "com.apple.safari")];
        let list = build_list(&[], &installed, Some("saf"));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Safari");
    }

    #[test]
    fn lista_corta_a_treinta() {
        let installed: Vec<AppRef> = (0..40).map(|i| app(&format!("App{i}"), &format!("id{i}"))).collect();
        assert_eq!(build_list(&[], &installed, None).len(), MAX_APP_LIST);
    }
}
```

- [ ] **Step 3: Ejecutar para ver que falla**

Run: `cargo test -p atlas-mac --lib apps`
Expected: FAIL (funciones aún sin implementar o sin compilar el resto de `apps.rs`).

- [ ] **Step 4: Implementar `handle_apps` y los textos estables**

> **Nota de corrección (fix round T2, commit `4ac2d02`):** `handle_apps` devuelve `crate::Result<OutputPayload>` y cada `OutputPayload::text(...)` usa `?` en vez de `.unwrap()`; `OutputPayload::Apps(...)` va envuelto en `Ok(...)`. El cuerpo de ejemplo abajo conserva los `.unwrap()` históricos por claridad; la implementación real va con `?`.


Añade a `apps.rs`:

```rust
pub fn handle_apps(bridge: &dyn AppBridge, payload: &AppsPayload) -> OutputPayload {
    match payload.action {
        AppsAction::List => {
            let apps = build_list(&bridge.running(), &bridge.installed(), payload.query.as_deref());
            OutputPayload::Apps(AppsOutput { apps })
        }
        AppsAction::Open => {
            let catalog = bridge.installed();
            match find_installed(&catalog, payload.name.as_deref(), payload.bundle_id.as_deref()) {
                None => OutputPayload::text(format!(
                    "no encuentro «{}» entre las apps instaladas", payload.name.as_deref().unwrap_or("?"))).unwrap(),
                Some(target) => match bridge.open(target) {
                    Ok(()) => OutputPayload::text(format!("Abierta «{}»", target.name)).unwrap(),
                    Err(err) => OutputPayload::text(format!("no se pudo abrir «{}»: {err}", target.name)).unwrap(),
                },
            }
        }
        AppsAction::Close => {
            let target = find_running(&bridge.running(), payload.name.as_deref(), payload.bundle_id.as_deref())
                .or_else(|| find_installed(&bridge.installed(), payload.name.as_deref(), payload.bundle_id.as_deref()));
            let Some(target) = target else {
                return OutputPayload::text(format!(
                    "«{}» no está abierta", payload.name.as_deref().unwrap_or("?"))).unwrap();
            };
            match bridge.terminate(target) {
                Err(err) => OutputPayload::text(format!("no se pudo cerrar «{}»: {err}", target.name)).unwrap(),
                Ok(CloseOutcome::Gone) => OutputPayload::text(format!("Cerrada «{}»", target.name)).unwrap(),
                Ok(CloseOutcome::StillPresent) => OutputPayload::text(format!(
                    "«{}» sigue abierta (¿tiene cambios sin guardar?); no la fuerzo", target.name)).unwrap(),
            }
        }
    }
}

pub fn find_running<'a>(running: &'a [AppRef], name: Option<&str>, bundle: Option<&str>) -> Option<&'a AppRef> {
    find_installed(running, name, bundle)
}
```

Asegúrate de `pub use` en `lib.rs`: `pub mod apps;` y `pub use apps::{AppBridge, AppRef, CloseOutcome, handle_apps};`.

- [ ] **Step 5: Escribir test de `handle_apps` con un FakeBridge**

Añade al `mod tests`:

```rust
    struct FakeBridge {
        running: Vec<AppRef>,
        installed: Vec<AppRef>,
        open_result: Result<(), String>,
        close_result: Result<CloseOutcome, String>,
    }
    impl AppBridge for FakeBridge {
        fn running(&self) -> Vec<AppRef> { self.running.clone() }
        fn installed(&self) -> Vec<AppRef> { self.installed.clone() }
        fn open(&self, _: &AppRef) -> Result<(), String> { self.open_result.clone() }
        fn terminate(&self, _: &AppRef) -> Result<CloseOutcome, String> { self.close_result.clone() }
    }
    fn payload(action: AppsAction, name: Option<&str>, query: Option<&str>) -> AppsPayload {
        AppsPayload { action, name: name.map(str::to_owned), bundle_id: None, query: query.map(str::to_owned) }
    }

    #[test]
    fn open_exitoso_da_texto_abierta() {
        let bridge = FakeBridge { running: vec![], installed: vec![app("Spotify", "com.spotify.client")], open_result: Ok(()), close_result: Ok(CloseOutcome::Gone) };
        let out = handle_apps(&bridge, &payload(AppsAction::Open, Some("Spotify"), None));
        assert_eq!(out.text_value(), Some("Abierta «Spotify»"));
    }

    #[test]
    fn open_sin_encontrar_no_abre() {
        let bridge = FakeBridge { running: vec![], installed: vec![app("Mail", "com.apple.mail")], open_result: Ok(()), close_result: Ok(CloseOutcome::Gone) };
        let out = handle_apps(&bridge, &payload(AppsAction::Open, Some("Gimp"), None));
        assert!(out.text_value().unwrap().contains("no encuentro"));
    }

    #[test]
    fn close_sigue_abierta_no_fuerza() {
        let bridge = FakeBridge { running: vec![app("Mail", "com.apple.mail")], installed: vec![], open_result: Ok(()), close_result: Ok(CloseOutcome::StillPresent) };
        let out = handle_apps(&bridge, &payload(AppsAction::Close, Some("Mail"), None));
        assert!(out.text_value().unwrap().contains("no la fuerzos") || out.text_value().unwrap().contains("sigue abierta"));
    }

    #[test]
    fn list_devuelve_apps_no_texto() {
        let bridge = FakeBridge { running: vec![app("Spotify", "com.spotify.client")], installed: vec![], open_result: Ok(()), close_result: Ok(CloseOutcome::Gone) };
        match handle_apps(&bridge, &payload(AppsAction::List, None, None)) {
            OutputPayload::Apps(out) => assert_eq!(out.apps[0].bundle_id, "com.spotify.client"),
            _ => panic!("esperaba OutputPayload::Apps"),
        }
    }
```

- [ ] **Step 6: Ejecutar y pasar**

Run: `cargo test -p atlas-mac --lib apps`
Expected: PASS.

Run: `cargo clippy -p atlas-mac --all-targets -- -D warnings && cargo fmt -p atlas-mac -- --check`
Expected: sin avisos.

- [ ] **Step 7: Commit**

```bash
git add crates/atlas-mac/src/apps.rs crates/atlas-mac/src/lib.rs crates/atlas-mac/src/wire.rs
git commit -m "feat(hefesto): módulo apps con resolución por nombre y trait AppBridge"
```

---

### Task 3: Despacho `hefesto.apps` en el ejecutor con puente inyectado

**Files:**
- Modify: `crates/atlas-mac/src/executor.rs:79-102` (campo `app_bridge`), `:326-357` (rama `ToolId::Apps`), builder `with_app_bridge`
- Modify: `crates/atlas-mac/src/runtime.rs:205-227,344` (campo `app_bridge`, `with_app_bridge`, pasarlo al `Executor::new`)
- Test: `crates/atlas-mac/tests/executor.rs`

**Interfaces:**
- Consumes: `crate::apps::handle_apps`, `AppBridge`, `AppsPayload` (Task 2).
- Produces: `Executor::with_app_bridge(self, std::sync::Arc<dyn AppBridge>) -> Self`; `Runtime::with_app_bridge(self, std::sync::Arc<dyn AppBridge>) -> Self`. Cuando no hay puente, `hefesto.apps` responde `Hefesto no puede gestionar apps` (texto), sin error de protocolo.

- [ ] **Step 1: Escribir el test de despacho con FakeBridge**

En `crates/atlas-mac/tests/executor.rs` reutiliza los helpers ya presentes (`id`, `at`, `status`, `invocation` con `generation(2)`, y la secuencia `prepare_registration → acknowledge_registration → prepare_resume → acknowledge_resume` de `dispatcher_routes_read_tools_and_rejects_mismatches`). Añade al final del fichero:

```rust
use std::sync::Arc;

struct FakeBridge;
impl atlas_mac::AppBridge for FakeBridge {
    fn running(&self) -> Vec<atlas_mac::AppRef> {
        vec![atlas_mac::AppRef {
            name: "Spotify".into(),
            bundle_id: "com.spotify.client".into(),
            url: "file:///Applications/Spotify.app".into(),
        }]
    }
    fn installed(&self) -> Vec<atlas_mac::AppRef> { self.running() }
    fn open(&self, _: &atlas_mac::AppRef) -> Result<(), String> { Ok(()) }
    fn terminate(&self, _: &atlas_mac::AppRef) -> Result<atlas_mac::CloseOutcome, String> {
        Ok(atlas_mac::CloseOutcome::Gone)
    }
}

fn apps_executor(bridge: Option<Arc<dyn atlas_mac::AppBridge>>) -> Executor {
    let mut executor = Executor::new(
        id("device"), id("instance"), Roots::new(vec![std::env::temp_dir()]));
    if let Some(bridge) = bridge { executor = executor.with_app_bridge(bridge); }
    let attempt = executor.prepare_registration(at(0)).unwrap();
    executor.acknowledge_registration(attempt, &status(1, true), at(0)).unwrap();
    let attempt = executor.prepare_resume(at(1)).unwrap();
    executor.acknowledge_resume(attempt, &status(2, false), at(2)).unwrap();
    executor
}

fn apps_invocation(tool: ToolId, payload: AppsPayload, inv_id: &str) -> Invocation {
    let mut inv = invocation();               // generation(2), device/instance correctos
    inv.invocation_id = id(inv_id);
    inv.tool_id = tool;
    inv.arguments = ToolArguments::Apps(payload);
    inv
}

#[test]
fn apps_open_close_and_list_route_through_bridge() {
    let mut executor = apps_executor(Some(Arc::new(FakeBridge)));
    let open = executor.execute(
        &apps_invocation(ToolId::Apps,
            AppsPayload { action: AppsAction::Open, name: Some("Spotify".into()),
                          bundle_id: None, query: None }, "a-open"),
        at(3)).unwrap();
    assert_eq!(open.output.text_value(), Some("Abierta «Spotify»"));

    let mut executor = apps_executor(Some(Arc::new(FakeBridge)));
    let close = executor.execute(
        &apps_invocation(ToolId::Apps,
            AppsPayload { action: AppsAction::Close, name: Some("Spotify".into()),
                          bundle_id: None, query: None }, "a-close"),
        at(3)).unwrap();
    assert_eq!(close.output.text_value(), Some("Cerrada «Spotify»"));

    let mut executor = apps_executor(Some(Arc::new(FakeBridge)));
    let list = executor.execute(
        &apps_invocation(ToolId::Apps,
            AppsPayload { action: AppsAction::List, name: None,
                          bundle_id: None, query: None }, "a-list"),
        at(3)).unwrap();
    match list.output {
        OutputPayload::Apps(out) => assert_eq!(out.apps[0].bundle_id, "com.spotify.client"),
        _ => panic!("esperaba OutputPayload::Apps"),
    }
}

#[test]
fn apps_sin_puente_devuelve_texto_no_disponible() {
    let mut executor = apps_executor(None);
    let out = executor.execute(
        &apps_invocation(ToolId::Apps,
            AppsPayload { action: AppsAction::Open, name: Some("Spotify".into()),
                          bundle_id: None, query: None }, "a-none"),
        at(3)).unwrap();
    assert_eq!(out.output.text_value(), Some("Hefesto no puede gestionar apps"));
}
```

- [ ] **Step 2: Ejecutar para ver que falla**

Run: `cargo test -p atlas-mac --test executor apps`
Expected: FAIL (`with_app_bridge` y rama `ToolId::Apps` no existen).

- [ ] **Step 3: Añadir campo y builder al `Executor`**

En `executor.rs`, campo `app_bridge: Option<std::sync::Arc<dyn crate::apps::AppBridge>>` inicializado a `None` en `new`, y:

```rust
pub fn with_app_bridge(mut self, bridge: std::sync::Arc<dyn crate::apps::AppBridge>) -> Self {
    self.app_bridge = Some(bridge);
    self
}
```

En el `match` de `execute()` (línea ~357, antes del `_ =>`), añade:

```rust
(ToolId::Apps, ToolArguments::Apps(payload)) => match &self.app_bridge {
            Some(bridge) => crate::apps::handle_apps(bridge.as_ref(), payload)?,
    None => OutputPayload::text("Hefesto no puede gestionar apps")?,
},
```

- [ ] **Step 4: Añadir puente al `Runtime` y pasarlo al ejecutor**

En `runtime.rs`, campo `app_bridge: Option<std::sync::Arc<dyn crate::apps::AppBridge>>` (None en `new`), builder `with_app_bridge`, y en la construcción del ejecutor (línea ~344):

```rust
let mut executor = Executor::new(self.device_id.clone(), instance.clone(), self.roots.clone());
if let Some(bridge) = &self.app_bridge {
    executor = executor.with_app_bridge(bridge.clone());
}
```

- [ ] **Step 5: Ejecutar y pasar**

Run: `cargo test -p atlas-mac`
Expected: PASS (despacho con y sin puente).

Run: `cargo clippy -p atlas-mac --all-targets -- -D warnings && cargo fmt -p atlas-mac -- --check`
Expected: sin avisos.

- [ ] **Step 6: Commit**

```bash
git add crates/atlas-mac/src/executor.rs crates/atlas-mac/src/runtime.rs crates/atlas-mac/tests/executor.rs
git commit -m "feat(hefesto): despacho hefesto.apps en el ejecutor con puente inyectado"
```

---

### Task 4: `AppBridge` real en `src-tauri` con NSWorkspace en el hilo principal

**Files:**
- Modify: `src-tauri/Cargo.toml` (deps `objc2`, `objc2-app-kit` con features `NSWorkspace`/`NSRunningApplication`, `plist`; fijas en la última 0.x publicada)
- Create: `src-tauri/src/hefesto_apps.rs` (implementación `AppBridge` + escaneo cacheado + `run_on_main_thread`)
- Modify: `src-tauri/src/lib.rs:346` (`Runtime::new(...).with_app_bridge(...)`)

**Interfaces:**
- Consumes: `atlas_mac::{AppBridge, AppRef, CloseOutcome}` (Task 2), `tauri::AppHandle` (hilo principal).
- Produces: `pub struct WorkspaceApps { handle: tauri::AppHandle, catalog: std::sync::Mutex<Option<Vec<AppRef>>> }` que implementa `AppBridge`. Se construye con `WorkspaceApps::new(app_handle)`.

- [ ] **Step 1: Añadir dependencias**

En `src-tauri/Cargo.toml`, sección `[dependencies]`:

```toml
objc2 = "0.5"
objc2-app-kit = { version = "0.2", features = ["NSWorkspace", "NSRunningApplication", "NSObjCRuntime"] }
plist = "1"
```

- [ ] **Step 2: Implementar el escaneo de instaladas (sin AppKit, cacheado)**

En `hefesto_apps.rs`:

```rust
use std::path::{Path, PathBuf};
use atlas_mac::{AppBridge, AppRef, CloseOutcome};

fn app_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from(home).join("Applications"),
    ]
}

fn read_app(path: &Path) -> Option<AppRef> {
    let plist = path.join("Contents").join("Info.plist");
    let value = plist::Value::from_file(&plist).ok()?;
    let map = value.as_dictionary()?;
    let bundle_id = map.get("CFBundleIdentifier")?.as_string()?.to_string();
    let name = map
        .get("CFBundleDisplayName").and_then(|v| v.as_string())
        .or_else(|| map.get("CFBundleName").and_then(|v| v.as_string()))
        .map(str::to_string)
        .unwrap_or_else(|| path.file_stem()?.to_string_lossy().to_string());
    Some(AppRef { name, bundle_id, url: path.to_string_lossy().to_string() })
}

fn scan_installed() -> Vec<AppRef> {
    let mut out = Vec::new();
    for dir in app_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("app") {
                if let Some(app) = read_app(&path) { out.push(app); }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
```

- [ ] **Step 3: Implementar el puente con reenvío al hilo principal**

```rust
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tauri::AppHandle;
use objc2_app_kit::{NSWorkspace, NSRunningApplication};
use objc2_foundation::{NSRunningApplicationActivationPolicy, NSURL};

pub struct WorkspaceApps {
    handle: AppHandle,
    catalog: Mutex<Option<Vec<AppRef>>>,
}

impl WorkspaceApps {
    pub fn new(handle: AppHandle) -> Self {
        Self { handle, catalog: Mutex::new(None) }
    }
    fn catalog(&self) -> Vec<AppRef> {
        let mut guard = self.catalog.lock().unwrap();
        if guard.is_none() { *guard = Some(scan_installed()); }
        guard.clone().unwrap()
    }
}

impl AppBridge for WorkspaceApps {
    fn installed(&self) -> Vec<AppRef> { self.catalog() }

    fn running(&self) -> Vec<AppRef> {
        let (tx, rx) = mpsc::channel();
        let handle = self.handle.clone();
        let _ = handle.run_on_main_thread(move || {
            let ws = NSWorkspace::sharedWorkspace();
            let apps: Vec<AppRef> = ws.runningApplications().into_iter()
                .filter(|a| a.activationPolicy() == NSRunningApplicationActivationPolicy::Regular)
                .filter_map(|a| Some(AppRef {
                    name: a.localizedName()?.to_string(),
                    bundle_id: a.bundleIdentifier()?.to_string(),
                    url: a.bundleURL().map(|u| u.path()).unwrap_or_default().to_string_lossy().to_string(),
                }))
                .collect();
            let _ = tx.send(apps);
        });
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default()
    }

    fn open(&self, target: &AppRef) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        let handle = self.handle.clone();
        let url = target.url.clone();
        handle.run_on_main_thread(move || {
            let nsurl = unsafe { NSURL::fileURLWithPath(&url.into()) };
            let ok = NSWorkspace::sharedWorkspace().openFile(&nsurl);
            let _ = tx.send(if ok { Ok(()) } else { Err("NSWorkspace rechazó la apertura".to_string()) });
        }).map_err(|_| "no se pudo reenviar al hilo principal".to_string())?;
        rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "timeout del hilo principal".to_string())?
    }

    fn terminate(&self, target: &AppRef) -> Result<CloseOutcome, String> {
        let (tx, rx) = mpsc::channel();
        let handle = self.handle.clone();
        let bundle_id = target.bundle_id.clone();
        handle.run_on_main_thread(move || {
            let ws = NSWorkspace::sharedWorkspace();
            let found: Option<NSRunningApplication> = ws.runningApplications().into_iter()
                .find(|a| a.bundleIdentifier().map(|b| b.to_string()) == Some(bundle_id.clone()));
            let Some(app) = found else { let _ = tx.send(Ok(CloseOutcome::Gone)); return; };
            let _ = app.terminate();
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(250));
                let still = NSWorkspace::sharedWorkspace().runningApplications().into_iter()
                    .any(|a| a.bundleIdentifier().map(|b| b.to_string()) == Some(bundle_id.clone()));
                if !still { let _ = tx.send(Ok(CloseOutcome::Gone)); return; }
            }
            let _ = tx.send(Ok(CloseOutcome::StillPresent));
        }).map_err(|_| "no se pudo reenviar al hilo principal".to_string())?;
        rx.recv_timeout(Duration::from_secs(8)).map_err(|_| "timeout del hilo principal".to_string())?
    }
}
```

Nota: si la API exacta de `objc2-app-kit` difiere (nombres de métodos/feature), ajústala a la versión fijada en el Step 1 sin cambiar el contrato de `AppBridge`; el compilador (`cargo check`) es el oráculo aquí. Mantén la espera de cierre acotada y en el hilo principal.

- [ ] **Step 4: Cablear el puente en el arranque**

En `lib.rs` línea ~346, donde se construye el `Runtime`, encadena el builder y registra la exportación del módulo (`mod hefesto_apps;` y `use hefesto_apps::WorkspaceApps;`):

```rust
let runtime = Runtime::new(client, device_id, roots)
    .with_app_bridge(Arc::new(WorkspaceApps::new(app.clone())));
```

- [ ] **Step 5: Compilar (no hay unit test; AppKit exige entorno gráfico)**

Run: `set -a; . ./.env 2>/dev/null; set +a; cargo check --workspace`
Expected: PASS sin errores.

Run: `cargo clippy -p atlas-next --all-targets -- -D warnings`
Expected: sin avisos (salvo `unsafe` justificado; documenta el `unsafe` del paso de `NSURL`).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/hefesto_apps.rs src-tauri/src/lib.rs
git commit -m "feat(hefesto): AppBridge real con NSWorkspace en el hilo principal"
```

---

### Task 5: Journal `hefesto.apps` (Python) sin contenido sensible

**Files:**
- Modify: `server/hefesto.py:139-141` (whitelist toolId), zona de validación de argumentos (~147-171), zona de validación de `output` (~475-490), reinicio‑unknown (~260 y rama write)
- Test: `server/test_hefesto.py` (o el fichero de journal que exista)

**Interfaces:**
- Consumes: `protocolVersion == 4` (Task 1).
- Produces: submit acepta `toolId == "hefesto.apps"` con `arguments` de forma `{"action": "open"|"close"|"list", "name"?: str, "bundleId"?: str, "query"?: str}`; `next` entrega el invocation a la primera línea de dispositivo (como `file_info`); `result` acepta `output` `{"text": ...}` o `{"apps": [{"name","bundleId","running"}]}`; una invocación `hefesto.apps` despachada y no devuelta al reiniciar queda `unknown`, sin reejecución.

- [ ] **Step 1: Escribir el test fallido del flujo apps**

En el fichero de pruebas del journal, añade un caso tipo:

```python
def test_apps_submit_next_result(self):
    body = {"protocolVersion": 4, "submissionId": "s1", "requestId": "r1", "sessionId": "x",
            "taskId": "t1", "toolId": "hefesto.apps",
            "arguments": {"action": "open", "name": "Spotify"}}
    self.journal.dispatch(self.controller, "POST", "/v1/hefesto/submit", json.dumps(body).encode())
    nxt = self.journal.dispatch(self.device, "POST", "/v1/hefesto/next",
                               json.dumps({"protocolVersion": 4}).encode())
    self.assertEqual(nxt[1]["invocation"]["arguments"]["name"], "Spotify")
    res = {"protocolVersion": 4, "invocationId": nxt[1]["invocation"]["invocationId"],
           "instanceId": self.instance, "generation": self.generation,
           "output": {"text": "Abierta «Spotify»"}}
    self.journal.dispatch(self.device, "POST", "/v1/hefesto/result", json.dumps(res).encode())
    out = self.journal.dispatch(self.controller, "POST", "/v1/hefesto/outcome",
        json.dumps({"protocolVersion": 4, "invocationId": res["invocationId"]}).encode())
    self.assertEqual(out[1]["state"], "succeeded")
```

Añade además `test_apps_action_invalida_se_rechaza` (con `"action": "kill"` → 400) y `test_apps_lista_acepta_output_estructurado` (output `{"apps":[{"name":"Mail","bundleId":"com.apple.mail","running":True}]}` → `succeeded`).

- [ ] **Step 2: Ejecutar para ver que falla**

Run: `cd server && python3 -m unittest -k apps 2>&1 | tail -20`
Expected: FAIL (toolId no whitelisted; validaciones ausentes).

- [ ] **Step 3: Whitelist y validación de argumentos**

En la tupla de toolIds admitidos (líneas 139-141) añade `"hefesto.apps"`. En la cascada de validación de `arguments` (junto a `hefesto.write_file`/`hefesto.rename`) añade:

```python
        elif body["toolId"] == "hefesto.apps":
            action = arguments.get("action")
            if action not in ("open", "close", "list"):
                raise _Rejected("bad_action", 400)
            for key in ("name", "bundleId", "query"):
                if key in arguments and (type(arguments[key]) is not str or len(arguments[key]) > 256):
                    raise _Rejected("bad_argument", 400)
            if action in ("open", "close") and not (arguments.get("name") or arguments.get("bundleId")):
                raise _Rejected("missing_target", 400)
```

- [ ] **Step 4: Validación de `output` para apps**

En la rama de validación de `output` (donde hoy `hefesto.file_info` o el texto), acepta el formato `apps` estructurado; trata lo demás como texto estándar (mismo límite `MAX_OUTPUT_BYTES`).

- [ ] **Step 5: Reinicio → `unknown` sin reejecución**

Extiende la lógica que hoy marca las escrituras pendientes (`hefesto.write_file`) como `unknown` tras reinicio para cubrir también `hefesto.apps` (es una operación con efecto). Revisa las ramas write‑aware (~líneas 260, 448, 456) y amplía la condición de toolId a `in ("hefesto.write_file", "hefesto.rename", "hefesto.apps")` donde corresponda, sin persistir contenido (apps no usa `_contents`).

- [ ] **Step 6: Ejecutar y pasar**

Run: `cd server && python3 -m unittest -k apps -v`
Expected: PASS.

Run: `cd server && python3 -m unittest discover -p "test_*.py"`
Expected: sin regresiones.

- [ ] **Step 7: Commit**

```bash
git add server/hefesto.py server
git commit -m "feat(hefesto): journal v4 acepta hefesto.apps sin contenido"
```

---

### Task 6: Herramienta `hefesto_apps` en el relay (catálogo, delegación y burbuja)

**Files:**
- Modify: `server/atlas_relay.py` (`HEFESTO_APPS_TOOL`, `HEFESTO_TOOLS` ~266, `execute_tool` ~883, nuevo `_apps` y `_format_apps_list`)
- Test: `server/test_hefesto_delegation.py` (nueva clase `AppFlowTests` sobre `FakeJournal`)

**Interfaces:**
- Consumes: `_hefesto_submit`, `_hefesto_wait`, `_hefesto_note` (existing), `protocolVersion 4` (Task 1), `hefesto.apps` aceptado por el journal (Task 5).
- Produces: herramienta de catálogo `hefesto_apps` (`requiresConfirmation: False`); `execute_tool("hefesto_apps", args, job)` → string para el modelo; burbuja HEFESTO en `open`/`close` de éxito; `list` devuelve texto formateado sin burbuja.

- [ ] **Step 1: Escribir los tests de flujo**

En `test_hefesto_delegation.py`, clonando el arnés de `WriteFlowTests`:

```python
class AppFlowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-apps-")
        self.addCleanup(self.temp.cleanup)
        self.brain = make_brain(self.temp.name)
        self.journal = FakeJournal()
        self.brain._hefesto_journal = self.journal
        self.brain._hefesto_device = "mac-one"

    def test_open_emite_burbuja_y_devuelve_texto(self):
        sid = self.brain._active()["id"]
        self.journal.accept("inv-apps")
        self.journal.outcomes["inv-apps"] = {"state": "succeeded", "output": {"text": "Abierta «Spotify»"}}
        with mock.patch.object(self.brain, "ask_confirmation") as ask:
            out = self.brain.execute_tool("hefesto_apps", {"action": "open", "name": "Spotify"},
                                          {**JOB, "session_id": sid})
        self.assertEqual(out, "Abierta «Spotify»")
        ask.assert_not_called()
        notes = [e for e in self.brain.events if e.get("kind") == "message" and e.get("agent") == "hefesto"]
        self.assertEqual(len(notes), 1)
        self.assertEqual(notes[0]["content"], "Abierta «Spotify»")

    def test_list_no_emite_burbuja(self):
        sid = self.brain._active()["id"]
        self.journal.accept("inv-list")
        self.journal.outcomes["inv-list"] = {"state": "succeeded",
            "output": {"apps": [{"name": "Mail", "bundleId": "com.apple.mail", "running": True}]}}
        out = self.brain.execute_tool("hefesto_apps", {"action": "list"}, {**JOB, "session_id": sid})
        self.assertIn("Mail", out)
        notes = [e for e in self.brain.events if e.get("kind") == "message" and e.get("agent") == "hefesto"]
        self.assertEqual(notes, [])

    def test_action_invalida_no_delega(self):
        self.journal.accept("inv-x")
        with mock.patch.object(self.brain, "_hefesto_submit") as submit:
            out = self.brain.execute_tool("hefesto_apps", {"action": "kill"}, JOB)
        self.assertIn("inválida", out)
        submit.assert_not_called()

    def test_open_sin_nombre_no_delega(self):
        with mock.patch.object(self.brain, "_hefesto_submit") as submit:
            out = self.brain.execute_tool("hefesto_apps", {"action": "open"}, JOB)
        self.assertIn("name", out)
        submit.assert_not_called()
```

- [ ] **Step 2: Ejecutar para ver que fallan**

Run: `cd server && python3 -m unittest test_hefesto_delegation.AppFlowTests -v`
Expected: FAIL (herramienta desconocida).

- [ ] **Step 3: Añadir entrada de catálogo**

En `atlas_relay.py`, tras `HEFESTO_RENAME_TOOL`:

```python
HEFESTO_APPS_TOOL = {
    "id": "hefesto_apps",
    "risk": "external",
    "requiresConfirmation": False,
    "description": ("Gestiona aplicaciones del Mac con Hefesto: action=open lanza una "
                    "app instalada (por name o bundleId), action=close la cierra de forma "
                    "suave (sin descartar cambios), action=list enumera apps en ejecución y "
                    "disponibles (≤30). No abre rutas arbitrarias ni binarios."),
    "schema": {
        "type": "object",
        "properties": {
            "action": {"type": "string", "enum": ["open", "close", "list"],
                       "description": "open, close o list"},
            "name": {"type": "string", "description": "nombre visible de la app (open/close)"},
            "bundleId": {"type": "string", "description": "identificador de bundle (alternativa a name)"},
            "query": {"type": "string", "description": "filtro opcional para list"},
        },
        "required": ["action"],
    },
}
```

Y en `HEFESTO_TOOLS` añade `HEFESTO_APPS_TOOL`.

- [ ] **Step 4: Ruta en `execute_tool` y `_apps` + formateo**

En `execute_tool` añade:

```python
        if tool_id == "hefesto_apps":
            return self._apps(job, args)
```

Nuevo método:

```python
    def _apps(self, job: dict, args: dict) -> str:
        action = args.get("action")
        if action not in ("open", "close", "list"):
            return "action inválida (open, close o list)"
        name = args.get("name")
        bundle = args.get("bundleId")
        if action in ("open", "close") and not (
                (isinstance(name, str) and name) or (isinstance(bundle, str) and bundle)):
            return "falta name o bundleId para " + action
        arguments = {"action": action}
        for key, value in (("name", name), ("bundleId", bundle), ("query", args.get("query"))):
            if isinstance(value, str) and value:
                arguments[key] = value
        invocation_id, error = self._hefesto_submit("hefesto.apps", job, arguments)
        if error is not None:
            return error
        output = self._hefesto_wait(invocation_id, job, 20.0)
        if isinstance(output, str):
            return output
        if "apps" in output:
            return self._format_apps_list(output.get("apps") or [])
        text = str(output.get("text") or "")
        if action in ("open", "close") and (text.startswith("Abierta «") or text.startswith("Cerrada «")):
            self._hefesto_note(job["session_id"], text)
        return text

    def _format_apps_list(self, apps: list) -> str:
        if not apps:
            return "sin apps"
        lines = []
        for a in apps[:30]:
            if isinstance(a, dict):
                state = "en ejecución" if a.get("running") else "disponible"
                lines.append(f"«{a.get('name')}» [{a.get('bundleId')}] {state}")
        return "\n".join(lines) if lines else "sin apps"
```

- [ ] **Step 5: Ejecutar y pasar**

Run: `cd server && python3 -m unittest test_hefesto_delegation.AppFlowTests -v`
Expected: PASS (4 pruebas).

Run: `cd server && python3 -m unittest discover -p "test_*.py"`
Expected: sin regresiones.

- [ ] **Step 6: Commit**

```bash
git add server/atlas_relay.py server/test_hefesto_delegation.py
git commit -m "feat(hefesto): herramienta hefesto_apps en el relay con burbuja"
```

---

### Task 7: Órbita «Gestiona apps», contratos y registro

**Files:**
- Modify: `src/state/orbitPlan.ts:17-22` (`HEFESTO_PHASES`)
- Modify: `src/state/orbitPlan.test.ts` (caso `hefesto_apps`)
- Modify: `docs/contracts.md` (sección Hefesto v3→v4, filas de `hefesto.apps`, `AppRecord`/`AppsOutput`, y nota de que no hay burbuja en `list`)
- Modify: `docs/hefesto-plan.md` §5 (título v4) y §6 (marcar la puerta 7 con este corte de apps)
- Verificar: `src/types/contracts.ts` ya trae `Agent = 'atlas' | 'hefesto'` (sin cambio)
- Verificar: `crates/atlas-core` **no** define la versión de protocolo (vive en `crates/atlas-mac/src/wire.rs`); la puerta de v4 ya se movió en la Tarea 1. Aquí no hay cambio en atlas-core salvo que un fixture suyo cite `3` (confirmar con `rg -n protocolVersion crates/atlas-core`).

**Interfaces:**
- Consumes: `hefesto_apps` emite `session.state_changed` en `executing` con `message == "hefesto_apps"` (patrón de las escrituras).
- Produces: órbita violeta «Hefesto · Gestiona apps»; contrato documentado v4.

- [ ] **Step 1: Escribir el test de órbita fallido**

En `orbitPlan.test.ts` añade:

```ts
it('órbita violeta para gestionar apps', () => {
  const plan = planPhaseOrbit('executing', 'hefesto_apps')
  expect(plan?.title).toBe('Hefesto · Gestiona apps')
})
```

- [ ] **Step 2: Ejecutar para ver que falla**

Run: `npx vitest run src/state/orbitPlan.test.ts`
Expected: FAIL (etiqueta no definida → `plan?.title` undefined).

- [ ] **Step 3: Añadir la etiqueta**

En `orbitPlan.ts`, en `HEFESTO_PHASES`, añade `hefesto_apps: 'Hefesto · Gestiona apps',`.

- [ ] **Step 4: Ejecutar y pasar**

Run: `npx vitest run src/state/orbitPlan.test.ts`
Expected: PASS.

- [ ] **Step 5: Actualizar `docs/contracts.md`**

Sube la cabecera de la sección a «Hefesto v4 — eco, lectura, escritura y apps» y todos los `protocolVersion: 3` → `4` en tablas/ejemplos. Añade a la tabla de herramientas una fila `hefesto.apps` (`{action, name?, bundleId?, query?}` → `{text}` resumen o `{apps:[{name, bundleId, running}]}`) y a la unión de `output` el caso `{ apps: AppRecord[] }`. Deja constancia: el relay expone `hefesto_apps` con `requiresConfirmation: False`; `open`/`close` de éxito producen burbuja `agent: hefesto`, `list` no.

- [ ] **Step 6: Actualizar `docs/hefesto-plan.md`**

Cabecera §5 a v4 y añade una línea en §6 (puerta 7): «Corte 1 — control de apps: `hefesto.apps` open/close/list, trivial por adaptador, cierre solo suave, resolución híbrida NSWorkspace+escaneo cacheado, sin API privada. Pendientes cortes de teclado y ventanas.»

- [ ] **Step 7: Compilar/chequear y commit**

Run: `npm run build`
Expected: PASS (`tsc -b` sin errores; `Agent` ya existe).

```bash
git add src/state/orbitPlan.ts src/state/orbitPlan.test.ts docs/contracts.md docs/hefesto-plan.md
git commit -m "docs(hefesto): contrato v4 y órbita de gestión de apps"
```

---

### Task 8: Verificación global, despliegue y prueba en vivo

**Files:**
- Modify: `docs/implemented.md` (nueva entrada con commit exacto)
- Modify: `docs/implementation-plan.md` (marcar el corte de apps)
- Modify: `docs/hefesto-plan.md` §8 (registro de ejecución)

**Interfaces:**
- Consumes: Tasks 1‑7 terminadas y commiteadas.
- Produces: relay desplegado a v4, bundle reconstruido, matriz en vivo verde y registro actualizado.

- [ ] **Step 1: Suites completas**

Run: `cargo test --workspace`
Run: `cargo clippy --workspace --all-targets -- -D warnings`
Run: `cargo fmt --all -- --check`
Run: `cd server && python3 -m unittest discover -p "test_*.py"`
Run: `npm run test && npm run build`
Expected: todo verde; anota counts (Python ≥ las pruebas nuevas de journal + relay; frontend con el caso de apps).

- [ ] **Step 2: Cobertura del spec**

Repasa §3.1‑3.6 y §5 del spec y señala qué tarea cubre cada requisito. Confirma: una sola herramienta, trivial sin tarjeta, burbuja solo en open/close, list ≤30 sin burbuja, cierre solo suave, sin API privada, puente en hilo principal.

- [ ] **Step 3: Desplegar el relay (requiere permiso explícito del usuario)**

Una vez el usuario autorice: `./scripts/deploy-relay.sh` (commitea antes si toca). Verifica `curl http://atlas-hub.local:8765/v1/health` y `/v1/hefesto/status`.

- [ ] **Step 4: Re‑prueba en vivo con el usuario (con permiso)**

Matriz: «abre <app inocua>» → burbuja «Abierta …» y la app arranca; «cierra <app>» → «Cerrada …» o «sigue abierta (cambios), no la fuerzo»; «lista apps en ejecución» → texto ≤30 sin burbuja; «abre NoExiste» → «no encuentro …» sin abrir. Comprobar que tras cerrar no quedan procesos nuevos ni RAM añadida (regla del Mac).

- [ ] **Step 5: Bundle (si hay cambios de dispositivo)**

`set -a; . ./.env; set +a` y `npm run tauri build` (o `apps/panel-app` si aplica) con `APPLE_SIGNING_IDENTITY="Apple Development: …"`; `codesign --verify --deep --strict`; instalar con `ditto` en `~/Applications`.

- [ ] **Step 6: Registro**

En `docs/implemented.md` nueva entrada con el/los commit(s) exacto(s) de este corte y la matriz en vivo. En `docs/implementation-plan.md` marca «Control de apps (puerta 7, corte 1)». En `docs/hefesto-plan.md` §8 añade la línea de ejecución con counts de pruebas y despliegue.

- [ ] **Step 7: Commit final (con permiso)**

```bash
git add docs
git commit -m "docs(hefesto): registro del control de apps (puerta 7, corte 1)"
```
