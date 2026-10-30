# Escritura segura de Hefesto — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** añadir creación exclusiva, sobrescritura con plan verificado y mover/renombrar de archivos de texto dentro de las raíces autorizadas de Hefesto, con consentimiento en las tres operaciones y sin persistir contenido en el journal.

**Architecture:** protocolo v3 entre relay (journal en el servidor) y dispositivo (Rust). El LLM llama `hefesto_write_file` (`{path, content, mode}`) o `hefesto_rename` (`{from, to}`); el relay consulta `hefesto.file_info` (interna) para redactar la tarjeta con tamaño y sha256 actuales y, al aprobar, envía la escritura con `expectedHash` inyectado. El dispositivo resuelve contención canónica, escribe temporal + publicación atómica y rechaza cualquier cambio de hash. El contenido de una escritura vive solo en memoria del journal.

**Tech Stack:** Rust (`atlas-mac`, `sha2`), Python (`server/hefesto.py`, `server/atlas_relay.py`), React (sin cambios de UI), SQLite del journal.

**Spec:** `docs/superpowers/specs/2026-10-30-hefesto-escritura-design.md`

## Global Constraints

- `protocolVersion: 3` en todas las solicitudes POST y respuestas.
- `toolId` admitidos en `submit`: `hefesto.echo`, `hefesto.list_dir`, `hefesto.read_file`, `hefesto.write_file`, `hefesto.rename`, `hefesto.file_info`.
- Contenido de escritura: texto UTF-8 ≤16 384 bytes, sin NUL. Rutas ≤1024 B, absolutas, sin NUL. Cuerpo ≤128 KiB. `output.text` ≤64 KiB.
- El contenido de las escrituras **nunca** se guarda en sqlite ni en logs; solo vive en memoria para entregarlo al dispositivo.
- Crear nunca sobrescribe: temporal + `hard_link` (falla si existe). Sobrescribir exige `expectedHash` y es temporal + `rename` atómico. Mover es `rename` atómico en el mismo volumen.
- Contención canónica por componentes en raíces del usuario; enlaces rechazados; el padre debe existir.
- Confirmación en las tres operaciones con resumen exacto y riesgo `write`.
- Límites del dispositivo: hash de existentes ≤64 MiB para el plan; sin borrado, sin `chmod`, sin directorios nuevos, sin enlaces.
- Comentarios en código: solo cuando el porqué no sea obvio. Sin `unwrap` en rutas de usuario.

---

## Estructura de ficheros

- `crates/atlas-mac/src/wire.rs` — v3, `FileText`, `ContentHash`, `WritePayload`, `RenamePayload`, `ToolId` nuevos, `OutputPayload` como unión.
- `crates/atlas-mac/src/lib.rs` — `Error::InvalidHash`.
- `crates/atlas-mac/src/tools.rs` — `resolve_new`, `file_info`, `write_file`, `rename`, hashing sha256.
- `crates/atlas-mac/src/executor.rs` — despacho de los tres toolIds y salida tipada.
- `crates/atlas-mac/src/runtime.rs` — badge del eco con el nuevo enum de argumentos.
- `crates/atlas-mac/tests/wire.rs`, `tests/executor.rs`, `tests/relay_tls.rs` — fixtures v3 y nuevos casos.
- `server/hefesto.py` — v3, validación por herramienta, contenido en memoria, salida tipada.
- `server/test_hefesto.py` — pruebas del journal.
- `server/atlas_relay.py` — catálogo, plan, delegación síncrona y resúmenes.
- `server/test_hefesto_delegation.py` — pruebas del relay.
- `docs/contracts.md`, `docs/hefesto-plan.md`, `docs/implemented.md`, `docs/implementation-plan.md` — registro.

---

### Task 1: protocolo v3 y vocabulario wire

**Files:**
- Modify: `crates/atlas-mac/src/wire.rs`
- Modify: `crates/atlas-mac/src/lib.rs`
- Modify: `crates/atlas-mac/src/executor.rs` (solo la construcción de `OutputPayload`)
- Modify: `crates/atlas-mac/tests/wire.rs`, `crates/atlas-mac/tests/executor.rs:102,210,759,768`, `crates/atlas-mac/tests/relay_tls.rs:229`, `crates/atlas-mac/src/runtime.rs:736`
- Modify: `server/hefesto.py` (versión 3 en `_validate` y respuestas)
- Modify: `server/atlas_relay.py` (todos los literales `"protocolVersion": 2`: eco, lectura, control, sondeo y estado)
- Test: `crates/atlas-mac/tests/wire.rs`, `server/test_hefesto.py`

**Interfaces:**
- Consumes: nada nuevo.
- Produces: `ProtocolVersion` = 3; `MAX_WRITE_BYTES: usize = 16 * 1024`; `FileText::new(impl Into<String>) -> Result<FileText>`; `ContentHash::new(impl Into<String>) -> Result<ContentHash>`; `WritePayload { path: ToolPath, content: FileText, expected_hash: Option<ContentHash> }`; `RenamePayload { from: ToolPath, to: ToolPath }`; `ToolArguments::{Echo, Path, Write, Rename}`; `ToolId::{WriteFile, Rename, FileInfo}`; `OutputPayload::{Text(TextPayload), Info(FileInfoOutput)}` con `OutputPayload::text(impl Into<String>) -> Result<Self>` y `OutputPayload::text_value(&self) -> Option<&str>`; `FileInfoOutput { exists: bool, size: Option<u64>, sha256: Option<ContentHash> }`; `Error::InvalidHash` (`invalid_hash`).

- [ ] **Step 1: escribir las pruebas nuevas de wire (fallan)**

Añade a `crates/atlas-mac/tests/wire.rs` (usa `serde_json` como el resto del fichero):

```rust
#[test]
fn write_y_rename_se_parsean_con_limites() {
    let write: SubmitRequest = serde_json::from_str(
        r#"{"protocolVersion":3,"submissionId":"sub-1","requestId":"req-1","sessionId":"ses-1","taskId":"tsk-1","toolId":"hefesto.write_file","arguments":{"path":"/Users/x/Documents/n.txt","content":"hola","expectedHash":null}}"#,
    )
    .unwrap();
    assert_eq!(write.tool_id, ToolId::WriteFile);
    let rename: SubmitRequest = serde_json::from_str(
        r#"{"protocolVersion":3,"submissionId":"sub-2","requestId":"req-1","sessionId":"ses-1","taskId":"tsk-2","toolId":"hefesto.rename","arguments":{"from":"/Users/x/Documents/a.txt","to":"/Users/x/Documents/b.txt"}}"#,
    )
    .unwrap();
    assert_eq!(rename.tool_id, ToolId::Rename);
    let info: SubmitRequest = serde_json::from_str(
        r#"{"protocolVersion":3,"submissionId":"sub-3","requestId":"req-1","sessionId":"ses-1","taskId":"tsk-3","toolId":"hefesto.file_info","arguments":{"path":"/Users/x/Documents/a.txt"}}"#,
    )
    .unwrap();
    assert_eq!(info.tool_id, ToolId::FileInfo);
}

#[test]
fn write_rechaza_hash_contenido_y_version_invalidos() {
    let base = r#"{"protocolVersion":3,"submissionId":"sub-1","requestId":"req-1","sessionId":"ses-1","taskId":"tsk-1","toolId":"hefesto.write_file","arguments":{"path":"/Users/x/Documents/n.txt","content":"hola","expectedHash":"%s"}}"#;
    assert!(serde_json::from_str::<SubmitRequest>(&base.replace("%s", "ABC")).is_err());
    assert!(serde_json::from_str::<SubmitRequest>(&base.replace("%s", &"a".repeat(63))).is_err());
    let big = "x".repeat(16 * 1024 + 1);
    let content = format!(
        r#"{{"protocolVersion":3,"submissionId":"sub-1","requestId":"req-1","sessionId":"ses-1","taskId":"tsk-1","toolId":"hefesto.write_file","arguments":{{"path":"/Users/x/Documents/n.txt","content":"{big}","expectedHash":null}}}}"#
    );
    assert!(serde_json::from_str::<SubmitRequest>(&content).is_err());
    let v2 = base.replace("%s", "null").replace("\"protocolVersion\":3", "\"protocolVersion\":2");
    assert!(serde_json::from_str::<SubmitRequest>(&v2).is_err());
}

#[test]
fn la_salida_tipificada_se_parsea_y_se_serializa() {
    let info: ResultRequest = serde_json::from_str(
        r#"{"protocolVersion":3,"instanceId":"instance-1","generation":1,"invocationId":"inv-1","output":{"exists":true,"size":12,"sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}}"#,
    )
    .unwrap();
    assert!(matches!(info.output, OutputPayload::Info(_)));
    let text = OutputPayload::text("hola").unwrap();
    assert_eq!(text.text_value(), Some("hola"));
    let json = serde_json::to_string(&text).unwrap();
    assert_eq!(json, r#"{"text":"hola"}"#);
}
```

Sustituye en `tests/wire.rs` cada `"protocolVersion":2` por `3` (salvo el valor de prueba de versión inválida) y añade `json!(2)` a la lista de versiones inválidas de `check_strict` (líneas 33-42), que hoy solo prueba `1`, para que v2 quede cubierta como rechazada.

- [ ] **Step 2: ejecutar y ver el fallo**

Run: `cargo test -p atlas-mac --test wire`
Expected: errores de compilación (`WritePayload` no existe) y fallos de parseo.

- [ ] **Step 3: implementar el vocabulario y la versión 3**

En `wire.rs`:

```rust
pub const MAX_WRITE_BYTES: usize = 16 * 1024;
pub const MAX_HASH_HEX: usize = 64;
```

`ProtocolVersion`: acepta `3` en `try_from` y devuelve `3` en `From`.

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FileText(String);

impl FileText {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() > MAX_WRITE_BYTES || value.contains('\0') {
            return Err(Error::InvalidText);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for FileText {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<FileText> for String {
    fn from(value: FileText) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentHash(String);

impl ContentHash {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
        if value.len() != MAX_HASH_HEX || !value.bytes().all(hex) {
            return Err(Error::InvalidHash);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ContentHash {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<ContentHash> for String {
    fn from(value: ContentHash) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WritePayload {
    pub path: ToolPath,
    pub content: FileText,
    #[serde(deserialize_with = "required_nullable")]
    pub expected_hash: Option<ContentHash>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenamePayload {
    pub from: ToolPath,
    pub to: ToolPath,
}
```

`required_nullable` ya existe en el módulo; hace que `expectedHash` sea obligatorio aunque sea `null`.

`ToolArguments`:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolArguments {
    Echo(EchoPayload),
    Path(PathPayload),
    Write(WritePayload),
    Rename(RenamePayload),
}
```

`ToolId` gana:

```rust
    #[serde(rename = "hefesto.write_file")]
    WriteFile,
    #[serde(rename = "hefesto.rename")]
    Rename,
    #[serde(rename = "hefesto.file_info")]
    FileInfo,
```

`OutputPayload` pasa a unión con helpers:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextPayload {
    pub text: TextOutput,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileInfoOutput {
    pub exists: bool,
    #[serde(deserialize_with = "required_nullable")]
    pub size: Option<u64>,
    #[serde(deserialize_with = "required_nullable")]
    pub sha256: Option<ContentHash>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OutputPayload {
    Text(TextPayload),
    Info(FileInfoOutput),
}

impl OutputPayload {
    pub fn text(value: impl Into<String>) -> Result<Self> {
        Ok(Self::Text(TextPayload {
            text: TextOutput::new(value)?,
        }))
    }

    pub fn text_value(&self) -> Option<&str> {
        match self {
            Self::Text(payload) => Some(payload.text.as_str()),
            Self::Info(_) => None,
        }
    }
}
```

`Error::InvalidHash` en `lib.rs` con código `"invalid_hash"`.

Ajusta las construcciones:
- `executor.rs`: `let output = OutputPayload::text(text)?;`
- `tests/executor.rs`: `result.output.text_value().unwrap()` en las cuatro líneas indicadas.
- `tests/relay_tls.rs`: `changed_result.output = OutputPayload::text("changed").unwrap();`
- `runtime.rs` (test): `state.results[0].output.text_value().unwrap()`.

En Python:
- `server/hefesto.py`: todas las respuestas `"protocolVersion": 2` → `3` y la comprobación `body["protocolVersion"] != 2` → `!= 3`.
- `server/test_hefesto.py`: `encoded()` pasa a `{"protocolVersion": 3, ...}` y todas las aserciones `value["protocolVersion"] == 2` → `3`.
- `server/atlas_relay.py`: **todos** los literales `"protocolVersion": 2` (suscripción de eco, lectura, control de suspensión y estado) pasan a `3`; comprueba con `rg -n '"protocolVersion": 2' server/atlas_relay.py` que no queda ninguno.

- [ ] **Step 4: ejecutar pruebas**

Run: `cargo test -p atlas-mac && cd server && python3 -m unittest test_hefesto -v`
Expected: PASS (los nuevos tests de wire y la suite del journal en v3).

- [ ] **Step 5: commit**

```bash
git add crates/atlas-mac server/hefesto.py server/atlas_relay.py server/test_hefesto.py
git commit -m "feat(hefesto): protocolo v3 con vocabulario de escritura"
```

---

### Task 2: herramientas de escritura en el dispositivo

**Files:**
- Modify: `crates/atlas-mac/Cargo.toml` (añadir `sha2 = "0.10"`)
- Modify: `crates/atlas-mac/src/tools.rs`
- Test: `crates/atlas-mac/src/tools.rs` (`mod tests`)

**Interfaces:**
- Consumes: `Roots`, `Error`, `ToolPath` no hace falta aquí (el ejecutor pasa `&str`).
- Produces:
  - `pub const HASH_MAX_BYTES: u64 = 64 * 1024 * 1024;`
  - `pub struct FileInfo { pub exists: bool, pub size: Option<u64>, pub sha256: Option<String> }`
  - `impl Roots { pub fn resolve_new(&self, path: &str) -> Option<std::path::PathBuf> }`
  - `pub fn file_info(roots: &Roots, path: &str) -> std::result::Result<FileInfo, String>`
  - `pub fn write_file(roots: &Roots, path: &str, content: &str, expected_hash: Option<&str>) -> String`
  - `pub fn rename(roots: &Roots, from: &str, to: &str) -> String`
  - `pub fn sha256_hex(bytes: &[u8]) -> String`

- [ ] **Step 1: escribir las pruebas que fallan**

En `mod tests` de `tools.rs` (reutiliza `TempDir`):

```rust
#[test]
fn escribir_crea_archivo_nuevo_y_falla_si_existe() {
    let dir = TempDir::new("write");
    let roots = Roots::new(vec![dir.path().to_path_buf()]);
    let target = dir.path().join("n.txt");
    let out = write_file(&roots, target.to_str().unwrap(), "hola", None);
    assert!(out.starts_with("Escrito «"));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "hola");
    let again = write_file(&roots, target.to_str().unwrap(), "otro", None);
    assert_eq!(again, "ya existe");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "hola");
}

#[test]
fn sobrescribir_exige_el_hash_actual() {
    let dir = TempDir::new("overwrite");
    let roots = Roots::new(vec![dir.path().to_path_buf()]);
    let target = dir.path().join("n.txt");
    std::fs::write(&target, "viejo").unwrap();
    let hash = sha256_hex(b"viejo");
    let out = write_file(&roots, target.to_str().unwrap(), "nuevo", Some(&hash));
    assert!(out.starts_with("Escrito «"));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "nuevo");
    let stale = write_file(&roots, target.to_str().unwrap(), "tercero", Some(&hash));
    assert_eq!(stale, "el archivo cambió desde la confirmación");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "nuevo");
}

#[test]
fn escribir_fuera_de_raices_o_por_enlace_se_rechaza() {
    let dir = TempDir::new("wsafe");
    let outside = TempDir::new("wsafe-out");
    let roots = Roots::new(vec![dir.path().to_path_buf()]);
    assert_eq!(
        write_file(&roots, outside.path().join("x.txt").to_str().unwrap(), "x", None),
        "fuera de las raíces autorizadas"
    );
    std::os::unix::fs::symlink(outside.path().join("x.txt"), dir.path().join("link.txt")).unwrap();
    assert_eq!(
        write_file(&roots, dir.path().join("link.txt").to_str().unwrap(), "x", None),
        "fuera de las raíces autorizadas"
    );
}

#[test]
fn file_info_devuelve_tamano_y_hash_y_rechaza_enlaces() {
    let dir = TempDir::new("info");
    let roots = Roots::new(vec![dir.path().to_path_buf()]);
    let target = dir.path().join("f.txt");
    std::fs::write(&target, "abc").unwrap();
    let info = file_info(&roots, target.to_str().unwrap()).unwrap();
    assert_eq!(info.size, Some(3));
    assert_eq!(info.sha256.as_deref(), Some(sha256_hex(b"abc").as_str()));
    let missing = file_info(&roots, dir.path().join("nuevo.txt").to_str().unwrap()).unwrap();
    assert!(!missing.exists);
    std::os::unix::fs::symlink(&target, dir.path().join("link.txt")).unwrap();
    assert_eq!(
        file_info(&roots, dir.path().join("link.txt").to_str().unwrap()),
        Err("fuera de las raíces autorizadas".to_string())
    );
}

#[test]
fn renombrar_mueve_sin_pisar_y_dentro_de_raices() {
    let dir = TempDir::new("rename");
    let outside = TempDir::new("rename-out");
    let roots = Roots::new(vec![dir.path().to_path_buf()]);
    let from = dir.path().join("a.txt");
    let to = dir.path().join("b.txt");
    std::fs::write(&from, "hola").unwrap();
    let out = rename(&roots, from.to_str().unwrap(), to.to_str().unwrap());
    assert!(out.starts_with("Movido «"));
    assert!(!from.exists());
    assert_eq!(std::fs::read_to_string(&to).unwrap(), "hola");
    let other = dir.path().join("c.txt");
    std::fs::write(&other, "x").unwrap();
    assert_eq!(
        rename(&roots, other.to_str().unwrap(), to.to_str().unwrap()),
        "el destino ya existe"
    );
    assert_eq!(
        rename(&roots, other.to_str().unwrap(), outside.path().join("d.txt").to_str().unwrap()),
        "fuera de las raíces autorizadas"
    );
}

#[test]
fn sha256_coincide_con_los_vectores_de_referencia() {
    assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}
```

- [ ] **Step 2: ejecutar y ver el fallo**

Run: `cargo test -p atlas-mac --lib tools::`
Expected: FAIL de compilación (`sha256_hex` no existe).

- [ ] **Step 3: implementar**

En `Cargo.toml`: `sha2 = "0.10"`.

En `tools.rs`:

```rust
use sha2::{Digest, Sha256};
use std::io::Read;

pub const HASH_MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileInfo {
    pub exists: bool,
    pub size: Option<u64>,
    pub sha256: Option<String>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
```

En `impl Roots`:

```rust
    /// Resuelve una ruta de escritura: canonicaliza el padre, exige contención
    /// y rechaza `.`/`..` y destinos que ya son enlaces. El destino puede no existir.
    pub fn resolve_new(&self, path: &str) -> Option<PathBuf> {
        let candidate = Path::new(path);
        if !candidate.is_absolute() {
            return None;
        }
        if candidate.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        }) {
            return None;
        }
        let name = candidate.file_name()?.to_str()?.to_string();
        let parent = candidate.parent()?.canonicalize().ok()?;
        if !parent.is_dir() || !self.entries.iter().any(|root| parent.starts_with(root)) {
            return None;
        }
        let target = parent.join(name);
        match std::fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => None,
            _ => Some(target),
        }
    }
```

Funciones nuevas:

```rust
pub fn file_info(roots: &Roots, path: &str) -> std::result::Result<FileInfo, String> {
    let Some(target) = roots.resolve_new(path) else {
        return Err("fuera de las raíces autorizadas".to_string());
    };
    let meta = match std::fs::symlink_metadata(&target) {
        Ok(meta) => meta,
        Err(_) => return Ok(FileInfo { exists: false, size: None, sha256: None }),
    };
    if !meta.is_file() {
        return Err("no es un archivo regular".to_string());
    }
    if meta.len() > HASH_MAX_BYTES {
        return Err("demasiado grande para planificar".to_string());
    }
    let hash = sha256_file(&target).map_err(|_| "no se pudo leer".to_string())?;
    Ok(FileInfo { exists: true, size: Some(meta.len()), sha256: Some(hash) })
}

fn temp_path(parent: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    parent.join(format!(".atlas-write-{}-{nanos}.tmp", std::process::id()))
}

fn stage(parent: &Path, content: &str) -> std::result::Result<PathBuf, String> {
    let temp = temp_path(parent);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| "no se pudo escribir".to_string())?;
    use std::io::Write;
    if file.write_all(content.as_bytes()).is_err() || file.sync_all().is_err() {
        let _ = std::fs::remove_file(&temp);
        return Err("no se pudo escribir".to_string());
    }
    Ok(temp)
}

pub fn write_file(roots: &Roots, path: &str, content: &str, expected_hash: Option<&str>) -> String {
    let Some(target) = roots.resolve_new(path) else {
        return "fuera de las raíces autorizadas".to_string();
    };
    let Some(parent) = target.parent() else {
        return "fuera de las raíces autorizadas".to_string();
    };
    let Some(expected) = expected_hash else {
        if std::fs::symlink_metadata(&target).is_ok() {
            return "ya existe".to_string();
        }
        let temp = match stage(parent, content) {
            Ok(temp) => temp,
            Err(message) => return message,
        };
        let published = std::fs::hard_link(&temp, &target);
        let _ = std::fs::remove_file(&temp);
        return match published {
            Ok(()) => format!(
                "Escrito «{path}» ({} B, sha256 {})",
                content.len(),
                &sha256_hex(content.as_bytes())[..12]
            ),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                "ya existe".to_string()
            }
            Err(_) => "no se pudo escribir".to_string(),
        };
    };
    let meta = match std::fs::symlink_metadata(&target) {
        Ok(meta) => meta,
        Err(_) => return "el archivo ya no existe".to_string(),
    };
    if !meta.is_file() {
        return "no es un archivo regular".to_string();
    }
    if meta.len() > HASH_MAX_BYTES {
        return "demasiado grande para planificar".to_string();
    }
    match sha256_file(&target) {
        Ok(current) if current == expected => {}
        Ok(_) => return "el archivo cambió desde la confirmación".to_string(),
        Err(_) => return "no se pudo leer".to_string(),
    }
    let temp = match stage(parent, content) {
        Ok(temp) => temp,
        Err(message) => return message,
    };
    match std::fs::rename(&temp, &target) {
        Ok(()) => format!(
            "Escrito «{path}» ({} B, sha256 {})",
            content.len(),
            &sha256_hex(content.as_bytes())[..12]
        ),
        Err(_) => {
            let _ = std::fs::remove_file(&temp);
            "no se pudo escribir".to_string()
        }
    }
}

pub fn rename(roots: &Roots, from: &str, to: &str) -> String {
    let source_meta = match std::fs::symlink_metadata(Path::new(from)) {
        Ok(meta) => meta,
        Err(_) => return "el origen no existe".to_string(),
    };
    if source_meta.file_type().is_symlink() {
        return "no es un archivo regular".to_string();
    }
    let Some(source) = roots.resolve(from) else {
        return "fuera de las raíces autorizadas".to_string();
    };
    let Some(destination) = roots.resolve_new(to) else {
        return "fuera de las raíces autorizadas".to_string();
    };
    let meta = match std::fs::symlink_metadata(&source) {
        Ok(meta) => meta,
        Err(_) => return "el origen no existe".to_string(),
    };
    if !meta.is_file() {
        return "no es un archivo regular".to_string();
    }
    if std::fs::symlink_metadata(&destination).is_ok() {
        return "el destino ya existe".to_string();
    }
    match std::fs::rename(&source, &destination) {
        Ok(()) => format!("Movido «{from}» → «{to}» ({} B)", meta.len()),
        Err(error) if error.raw_os_error() == Some(18) => {
            "no se puede mover entre volúmenes".to_string()
        }
        Err(_) => "no se pudo mover".to_string(),
    }
}
```

- [ ] **Step 4: ejecutar pruebas y fmt**

Run: `cargo test -p atlas-mac --lib tools:: && cargo fmt -p atlas-mac -- --check`
Expected: 13 tests PASS (7 previos + 6 nuevos) y sin diffs de formato.

- [ ] **Step 5: commit**

```bash
git add crates/atlas-mac/Cargo.toml crates/atlas-mac/Cargo.lock crates/atlas-mac/src/tools.rs
git commit -m "feat(hefesto): creación, sobrescritura y renombrado seguros en el dispositivo"
```

---

### Task 3: journal v3 con contenido en memoria

**Files:**
- Modify: `server/hefesto.py`
- Test: `server/test_hefesto.py`

**Interfaces:**
- Consumes: protocolo v3 (Task 1).
- Produces: validación de argumentos por herramienta; `invocations.submission` redactado (`contentBytes`, `contentSha256`); contenido servido por `next` desde `self._contents`; `outcome` devuelve `output` tipado; reinicio sin contenido ⇒ `unknown`.

- [ ] **Step 1: escribir las pruebas que fallan**

Añade a `server/test_hefesto.py`:

```python
WRITE = {"submissionId": "submission-w", "requestId": "request-1", "sessionId": "session-1",
         "taskId": "task-1", "toolId": "hefesto.write_file",
         "arguments": {"path": "/Users/x/Documents/n.txt", "content": "hola",
                       "expectedHash": None}}
RENAME = {"submissionId": "submission-r", "requestId": "request-1", "sessionId": "session-1",
          "taskId": "task-2", "toolId": "hefesto.rename",
          "arguments": {"from": "/Users/x/Documents/a.txt", "to": "/Users/x/Documents/b.txt"}}
INFO = {"submissionId": "submission-i", "requestId": "request-1", "sessionId": "session-1",
        "taskId": "task-3", "toolId": "hefesto.file_info",
        "arguments": {"path": "/Users/x/Documents/a.txt"}}


class WriteJournalTests(JournalTests):
    def test_el_contenido_no_se_guarda_en_sqlite(self):
        self.ready()
        self.ok("submit", peer=CONTROLLER, **WRITE)
        invocation = self.next()
        self.assertEqual(invocation["arguments"]["content"], "hola")
        with sqlite3.connect(self.path) as db:
            raw = db.execute("SELECT submission FROM invocations").fetchone()[0]
        self.assertNotIn("hola", raw)
        self.assertIn("contentSha256", raw)

    def test_reinicio_sin_contenido_deja_unknown_y_no_reenvia(self):
        self.ready()
        self.ok("submit", peer=CONTROLLER, **WRITE)
        self.next()
        self.restart()
        self.ready(instance="instance-2")
        self.assertIsNone(self.next())

    def test_write_valida_argumentos_y_salida(self):
        self.ready()
        broken = json.loads(json.dumps(WRITE))
        broken["arguments"]["expectedHash"] = "XYZ"
        self.error("invalid_hash", "submit", peer=CONTROLLER, **broken)
        self.ok("submit", peer=CONTROLLER, **WRITE)
        invocation = self.next()
        status, value = self.ok(
            "result",
            **self.result_fields(invocation, output={"text": "Escrito «x» (4 B, sha256 abc)"}))
        self.assertEqual(value["state"], "succeeded")

    def test_file_info_admite_info_o_texto_de_rechazo(self):
        self.ready()
        self.ok("submit", peer=CONTROLLER, **INFO)
        invocation = self.next()
        self.ok("result",
                **self.result_fields(invocation, output={"exists": True, "size": 3,
                                                         "sha256": "0" * 64}))
        status, value = self.ok("outcome", peer=CONTROLLER,
                                invocationId=invocation["invocationId"])
        self.assertEqual(value["output"]["size"], 3)
        self.ok("submit", peer=CONTROLLER,
                **{**INFO, "submissionId": "submission-i2", "taskId": "task-4"})
        rejection = self.next()
        self.ok("result", **self.result_fields(
            rejection, output={"text": "fuera de las raíces autorizadas"}))

    def test_write_rechaza_la_salida_de_info(self):
        self.ready()
        self.ok("submit", peer=CONTROLLER, **WRITE)
        invocation = self.next()
        self.error("invalid_text", "result",
                   **self.result_fields(invocation, output={"exists": True, "size": 3,
                                                            "sha256": "0" * 64}))

    def test_submit_idempotente_no_duplica_y_conflicta_si_cambia_el_contenido(self):
        self.ready()
        first = self.ok("submit", peer=CONTROLLER, **WRITE)
        self.assertEqual(self.ok("submit", peer=CONTROLLER, **WRITE), first)
        changed = json.loads(json.dumps(WRITE))
        changed["arguments"]["content"] = "otro"
        self.error("submission_conflict", "submit", peer=CONTROLLER, **changed)

    def test_rename_valida_rutas(self):
        self.ready()
        broken = json.loads(json.dumps(RENAME))
        broken["arguments"]["to"] = "relativo"
        self.error("invalid_path", "submit", peer=CONTROLLER, **broken)
        self.ok("submit", peer=CONTROLLER, **RENAME)
```

- [ ] **Step 2: ejecutar y ver el fallo**

Run: `cd server && python3 -m unittest test_hefesto.WriteJournalTests -v`
Expected: FAIL (`unsupported_tool`, `invalid_fields`, `KeyError: 'output'`).

- [ ] **Step 3: implementar**

En `_validate`, tras la comprobación de versión:

```python
    if operation == "submit" and body["toolId"] not in (
            "hefesto.echo", "hefesto.list_dir", "hefesto.read_file",
            "hefesto.write_file", "hefesto.rename", "hefesto.file_info"):
        raise _Rejected("unsupported_tool")
    if operation == "submit":
        args = body["arguments"]
        if type(args) is not dict:
            raise _Rejected("invalid_fields")
        if body["toolId"] == "hefesto.echo":
            if set(args) != {"text"} or type(args["text"]) is not str:
                raise _Rejected("invalid_text")
            try:
                if len(args["text"].encode("utf-8")) > 256:
                    raise _Rejected("invalid_text")
            except UnicodeError:
                raise _Rejected("invalid_text") from None
        elif body["toolId"] == "hefesto.write_file":
            if set(args) != {"path", "content", "expectedHash"}:
                raise _Rejected("invalid_fields")
            _validate_path(args["path"])
            if type(args["content"]) is not str:
                raise _Rejected("invalid_text")
            try:
                if "\x00" in args["content"] or len(args["content"].encode("utf-8")) > 16 * 1024:
                    raise _Rejected("invalid_text")
            except UnicodeError:
                raise _Rejected("invalid_text") from None
            expected = args["expectedHash"]
            if expected is not None and (type(expected) is not str
                                         or len(expected) != 64
                                         or any(c not in "0123456789abcdef" for c in expected)):
                raise _Rejected("invalid_hash")
        elif body["toolId"] == "hefesto.rename":
            if set(args) != {"from", "to"}:
                raise _Rejected("invalid_fields")
            _validate_path(args["from"])
            _validate_path(args["to"])
        else:
            if set(args) != {"path"}:
                raise _Rejected("invalid_fields")
            _validate_path(args["path"])
```

Con un helper `_validate_path` extraído del bloque actual, definido justo antes de `_validate`:

```python
def _validate_path(value):
    if type(value) is not str:
        raise _Rejected("invalid_path")
    try:
        if not value.startswith("/") or "\x00" in value or len(value.encode("utf-8")) > 1024:
            raise _Rejected("invalid_path")
    except UnicodeError:
        raise _Rejected("invalid_path") from None
```

La validación genérica de `output` en `_validate` pasa a:

```python
    if "output" in body:
        value = body["output"]
        if type(value) is dict and set(value) == {"text"} and type(value["text"]) is str:
            try:
                if len(value["text"].encode("utf-8")) > 64 * 1024:
                    raise _Rejected("invalid_text")
            except UnicodeError:
                raise _Rejected("invalid_text") from None
        elif (type(value) is dict and set(value) == {"exists", "size", "sha256"}
              and type(value["exists"]) is bool):
            size, digest = value["size"], value["sha256"]
            ok_size = (size is None) if not value["exists"] else (
                type(size) is int and 0 <= size <= 2**63 - 1)
            ok_hash = (digest is None) if not value["exists"] else (
                type(digest) is str and len(digest) == 64
                and all(c in "0123456789abcdef" for c in digest))
            if not ok_size or not ok_hash:
                raise _Rejected("invalid_output")
        else:
            raise _Rejected("invalid_output")
```

En `Journal.__init__`: `self._contents = {}`.

El contenido en memoria se libera en cada transición terminal (aborto, caducidad y éxito). Sustituye `_cancel_active` y el bloque de expiración de `_sweep` por:

```python
    def _cancel_active(self, device_id):
        ids = [row["invocation_id"] for row in self._db.execute(
            "SELECT invocation_id FROM invocations WHERE device_id=? AND state IN ('pending','dispatched')",
            (device_id,)).fetchall()]
        self._db.execute("""UPDATE invocations SET state=CASE state
                         WHEN 'pending' THEN 'cancelled' ELSE 'unknown' END
                         WHERE device_id=? AND state IN ('pending','dispatched')""", (device_id,))
        for invocation_id in ids:
            self._contents.pop(invocation_id, None)
```

```python
    def _sweep(self, now, wall):
        for device in self._db.execute(
                "SELECT device_id FROM devices WHERE instance_id IS NOT NULL AND lease_mono<=?", (now,)).fetchall():
            self._cancel_active(device["device_id"])
            self._db.execute("""UPDATE devices SET instance_id=NULL, lease_mono=NULL,
                             generation=generation+1, suspended=1 WHERE device_id=?""", (device["device_id"],))
        expired = [row["invocation_id"] for row in self._db.execute(
            "SELECT invocation_id FROM invocations WHERE state IN ('pending','dispatched') "
            "AND (deadline_mono<=? OR expires_ms<=?)", (now, wall * 1000)).fetchall()]
        self._db.execute("""UPDATE invocations SET state=CASE state
                         WHEN 'pending' THEN 'expired' ELSE 'unknown' END
                         WHERE state IN ('pending','dispatched') AND (deadline_mono<=? OR expires_ms<=?)""",
                         (now, wall * 1000))
        for invocation_id in expired:
            self._contents.pop(invocation_id, None)
```

En `submit` (antes del INSERT):

```python
            arguments = data["arguments"]
            redacted = {key: value for key, value in data.items() if key != "arguments"}
            redacted["arguments"] = {key: value for key, value in arguments.items()
                                     if key != "content"}
            if "content" in arguments:
                encoded_content = arguments["content"].encode("utf-8")
                redacted["arguments"]["contentBytes"] = len(encoded_content)
                redacted["arguments"]["contentSha256"] = hashlib.sha256(encoded_content).hexdigest()
            submission = json.dumps(redacted, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
```

(La comparación de idempotencia por `submission` usa el JSON redactado, así que un contenido distinto con el mismo `submissionId` da `submission_conflict`.)

Tras el INSERT, guarda el contenido en memoria:

```python
            if "content" in data["arguments"]:
                self._contents[invocation_id] = data["arguments"]
```

La columna `text` del INSERT: `data["arguments"].get("text", "")` (las escrituras guardan `""`).

En `next`:

```python
            if active is not None:
                submitted = json.loads(active["submission"])
                arguments = submitted["arguments"]
                if submitted["toolId"] == "hefesto.write_file":
                    # El contenido no persiste: si el proceso se reinició entre
                    # submit y next, la escritura queda `unknown` y no se entrega.
                    arguments = self._contents.get(active["invocation_id"])
                    if arguments is None:
                        self._db.execute(
                            "UPDATE invocations SET state='unknown' WHERE invocation_id=?",
                            (active["invocation_id"],))
                        return {"protocolVersion": 3, "invocation": None}
                self._db.execute("UPDATE invocations SET state='dispatched' WHERE invocation_id=?",
                                 (active["invocation_id"],))
                invocation = {"protocolVersion": 3, "invocationId": active["invocation_id"],
                              "requestId": active["request_id"], "sessionId": active["session_id"],
                              "taskId": active["task_id"], "deviceId": device_id,
                              "instanceId": active["instance_id"], "generation": active["generation"],
                              "toolId": submitted["toolId"], "arguments": arguments,
                              "expiresAtUnixMs": active["expires_ms"]}
```

En `result`, la validación por herramienta:

```python
        submitted = json.loads(invocation["submission"])
        tool = submitted["toolId"]
        output = data["output"]
        if tool == "hefesto.file_info":
            # La info puede degradar a texto cuando el dispositivo rechaza la
            # ruta o el archivo; el relay lo interpreta como error del plan.
            info_shape = set(output) == {"exists", "size", "sha256"}
            text_shape = set(output) == {"text"} and type(output["text"]) is str
            if not (info_shape or text_shape):
                raise _Rejected("invalid_output")
        elif set(output) != {"text"}:
            raise _Rejected("invalid_text")
        if tool == "hefesto.echo" and output["text"] != invocation["text"]:
            raise _Rejected("result_conflict", 409)
        # Mismo JSON canónico que se persiste, o un reintento tras ACK perdido
        # compararía formatos distintos y daría conflicto.
        canonical = json.dumps(output, sort_keys=True, separators=(",", ":"))
        if invocation["state"] == "succeeded":
            if canonical != invocation["output_text"]:
                raise _Rejected("result_conflict", 409)
            return self._ack(invocation)
        ...
        self._db.execute(
            "UPDATE invocations SET state='succeeded', output_text=? WHERE invocation_id=?",
            (canonical, invocation["invocation_id"]))
        self._contents.pop(invocation["invocation_id"], None)
```

En `outcome`, devuelve `output`:

```python
            output = None
            if invocation["output_text"] is not None:
                try:
                    output = json.loads(invocation["output_text"])
                except ValueError:
                    output = {"text": invocation["output_text"]}
            return {"protocolVersion": 3, "invocationId": invocation["invocation_id"],
                    "state": invocation["state"], "output": output}
```

- [ ] **Step 4: ejecutar la suite Python**

Run: `cd server && python3 -m unittest discover -p "test_*.py"`
Expected: PASS. Los tests existentes que leían `outputText` se actualizan en este paso a `output["text"]`; los que construían `output={"text": ...}` siguen valiendo.

- [ ] **Step 5: commit**

```bash
git add server/hefesto.py server/test_hefesto.py
git commit -m "feat(hefesto): journal v3 con contenido en memoria y salida tipada"
```

---

### Task 4: despacho en el dispositivo

**Files:**
- Modify: `crates/atlas-mac/src/executor.rs`
- Modify: `crates/atlas-mac/src/runtime.rs` (badge del eco y tests)
- Test: `crates/atlas-mac/tests/executor.rs`, `crates/atlas-mac/tests/relay_tls.rs`

**Interfaces:**
- Consumes: Task 1 (wire) y Task 2 (`tools::write_file/rename/file_info`).
- Produces: `Executor::execute` enruta `WriteFile`/`Rename`/`FileInfo`; `file_info` devuelve `OutputPayload::Info` o `OutputPayload::text(error)`; el badge solo aplica al eco.

- [ ] **Step 1: escribir la prueba de despacho**

Añade a `crates/atlas-mac/tests/executor.rs` (junto al test de lectura; usa su helper `test_roots()` y el patrón de invocación existente):

```rust
#[test]
fn dispatcher_ejecuta_escritura_renombrado_e_info() {
    let dir = std::env::temp_dir().join(format!(
        "atlas-dispatch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut executor = Executor::new(id("device"), id("instance"), Roots::new(vec![dir.clone()]));
    let attempt = executor.prepare_registration(at(0)).unwrap();
    executor
        .acknowledge_registration(attempt, &status(1, true), at(0))
        .unwrap();
    let attempt = executor.prepare_resume(at(1)).unwrap();
    executor
        .acknowledge_resume(attempt, &status(2, false), at(2))
        .unwrap();

    let target = dir.join("n.txt");
    let mut task = invocation();
    task.generation = generation(2);
    task.tool_id = ToolId::WriteFile;
    task.arguments = ToolArguments::Write(WritePayload {
        path: ToolPath::new(target.to_str().unwrap()).unwrap(),
        content: FileText::new("hola").unwrap(),
        expected_hash: None,
    });
    let result = executor.execute(&task, at(3)).unwrap();
    assert!(result.output.text_value().unwrap().starts_with("Escrito «"));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "hola");

    let renamed = dir.join("m.txt");
    let mut task = invocation();
    task.generation = generation(2);
    task.invocation_id = id("invocation-2");
    task.tool_id = ToolId::Rename;
    task.arguments = ToolArguments::Rename(RenamePayload {
        from: ToolPath::new(target.to_str().unwrap()).unwrap(),
        to: ToolPath::new(renamed.to_str().unwrap()).unwrap(),
    });
    let result = executor.execute(&task, at(4)).unwrap();
    assert!(result.output.text_value().unwrap().starts_with("Movido «"));
    assert!(renamed.exists());

    let mut task = invocation();
    task.generation = generation(2);
    task.invocation_id = id("invocation-3");
    task.tool_id = ToolId::FileInfo;
    task.arguments = ToolArguments::Path(PathPayload {
        path: ToolPath::new(renamed.to_str().unwrap()).unwrap(),
    });
    let result = executor.execute(&task, at(5)).unwrap();
    assert!(matches!(result.output, OutputPayload::Info(ref info) if info.exists));
    std::fs::remove_dir_all(&dir).unwrap();
}
```

- [ ] **Step 2: ejecutar y ver el fallo**

Run: `cargo test -p atlas-mac --test executor dispatcher_ejecuta`
Expected: FAIL (`InvalidArguments` para los toolIds nuevos).

- [ ] **Step 3: implementar el despacho**

En `Executor::execute`:

```rust
        let output = match (&invocation.tool_id, &invocation.arguments) {
            (ToolId::Echo, ToolArguments::Echo(payload)) => {
                OutputPayload::text(payload.text.as_str())?
            }
            (ToolId::ListDir, ToolArguments::Path(payload)) => {
                OutputPayload::text(crate::tools::list_dir(&self.roots, payload.path.as_str()))?
            }
            (ToolId::ReadFile, ToolArguments::Path(payload)) => {
                OutputPayload::text(crate::tools::read_file(&self.roots, payload.path.as_str()))?
            }
            (ToolId::WriteFile, ToolArguments::Write(payload)) => OutputPayload::text(
                crate::tools::write_file(
                    &self.roots,
                    payload.path.as_str(),
                    payload.content.as_str(),
                    payload.expected_hash.as_ref().map(|hash| hash.as_str()),
                ),
            )?,
            (ToolId::Rename, ToolArguments::Rename(payload)) => OutputPayload::text(
                crate::tools::rename(&self.roots, payload.from.as_str(), payload.to.as_str()),
            )?,
            (ToolId::FileInfo, ToolArguments::Path(payload)) => {
                match crate::tools::file_info(&self.roots, payload.path.as_str()) {
                    Ok(info) => OutputPayload::Info(FileInfoOutput {
                        exists: info.exists,
                        size: info.size,
                        sha256: info.sha256.as_deref().map(ContentHash::new).transpose()?,
                    }),
                    Err(text) => OutputPayload::text(text)?,
                }
            }
            _ => return Err(Error::InvalidArguments),
        };
```

En `runtime.rs`, el badge del sondeo:

```rust
        let badge = match &invocation.arguments {
            ToolArguments::Echo(payload) => Some(payload.text.as_str().to_string()),
            _ => None,
        };
```

Actualiza `tests/relay_tls.rs` y cualquier construcción de `OutputPayload` que haya quedado.

- [ ] **Step 4: ejecutar toda la suite Rust**

Run: `cargo test --workspace && cargo clippy -p atlas-mac --all-targets -- -D warnings && cargo fmt -p atlas-mac -- --check`
Expected: PASS.

- [ ] **Step 5: commit**

```bash
git add crates/atlas-mac
git commit -m "feat(hefesto): despacho de escritura, renombrado e info en el ejecutor"
```

---

### Task 5: relay — catálogo, plan y delegación

**Files:**
- Modify: `server/atlas_relay.py`
- Test: `server/test_hefesto_delegation.py`

**Interfaces:**
- Consumes: journal v3 y `outcome.output` (Task 3).
- Produces:
  - `HEFESTO_WRITE_TOOL` (`id: hefesto_write_file`, `requiresConfirmation: True`, `risk: write`) con schema `{path, content, mode}`.
  - `HEFESTO_RENAME_TOOL` (`id: hefesto_rename`) con schema `{from, to}`.
  - `Brain._hefesto_submit(tool_id, job, arguments) -> (invocation_id | None, error_text | None)`
  - `Brain._hefesto_wait(invocation_id, job, timeout) -> dict | str`
  - `Brain._file_info(path, job) -> dict` (Info o `{"error": texto}`)
  - `Brain._write_file(job, args) -> str`, `Brain._rename_file(job, args) -> str`
  - `ask_confirmation(..., summary: tuple[str, str] | None = None)`

- [ ] **Step 1: escribir las pruebas que fallan**

Añade a `server/test_hefesto_delegation.py`:

```python
class WriteFlowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-write-")
        self.addCleanup(self.temp.cleanup)
        self.brain = make_brain(self.temp.name)
        self.journal = FakeJournal()
        self.brain._hefesto_journal = self.journal
        self.brain._hefesto_device = "mac-one"

    def info(self, exists, size=None, sha=None):
        return {"exists": exists, "size": size, "sha256": sha}

    def test_crear_pide_tarjeta_y_escribe_sin_hash(self):
        journal = self.journal
        journal.accept("inv-info")
        journal.outcomes["inv-info"] = {"state": "succeeded",
                                        "output": self.info(False)}
        journal.accept("inv-write")
        journal.outcomes["inv-write"] = {"state": "succeeded",
                                          "output": {"text": "Escrito «x»"}}
        with mock.patch.object(self.brain, "ask_confirmation",
                               return_value="allow_once") as ask:
            result = self.brain.execute_tool(
                "hefesto_write_file",
                {"path": "/Users/x/Documents/n.txt", "content": "hola", "mode": "create"},
                JOB)
        self.assertEqual(result, "Escrito «x»")
        self.assertEqual(ask.call_args.kwargs["summary"], ("Crear «/Users/x/Documents/n.txt» (4 B)", "write"))
        self.assertEqual(journal.submits[1]["toolId"], "hefesto.write_file")
        self.assertEqual(journal.submits[1]["arguments"],
                         {"path": "/Users/x/Documents/n.txt", "content": "hola",
                          "expectedHash": None})

    def test_catalogo_incluye_escritura_y_no_expone_file_info(self):
        ids = [tool["id"] for tool in self.brain.tool_defs()]
        self.assertIn("hefesto_write_file", ids)
        self.assertIn("hefesto_rename", ids)
        self.assertNotIn("hefesto.file_info", ids)

    def test_crear_con_existente_no_abre_tarjeta(self):
        self.journal.accept("inv-info")
        self.journal.outcomes["inv-info"] = {"state": "succeeded",
                                             "output": self.info(True, 10, "a" * 64)}
        with mock.patch.object(self.brain, "ask_confirmation") as ask:
            result = self.brain.execute_tool(
                "hefesto_write_file",
                {"path": "/Users/x/Documents/n.txt", "content": "hola", "mode": "create"},
                JOB)
        self.assertIn("ya existe", result)
        ask.assert_not_called()

    def test_denegar_no_envia_la_escritura(self):
        self.journal.accept("inv-info")
        self.journal.outcomes["inv-info"] = {"state": "succeeded",
                                             "output": self.info(False)}
        with mock.patch.object(self.brain, "ask_confirmation", return_value="deny"):
            result = self.brain.execute_tool(
                "hefesto_write_file",
                {"path": "/Users/x/Documents/n.txt", "content": "hola", "mode": "create"},
                JOB)
        self.assertEqual(result, "denegada por el usuario")
        self.assertEqual(len(self.journal.submits), 1)
        self.assertEqual(self.journal.submits[0]["toolId"], "hefesto.file_info")

    def test_sobrescribir_inyecta_el_hash_del_plan(self):
        sha = "b" * 64
        self.journal.accept("inv-info")
        self.journal.outcomes["inv-info"] = {"state": "succeeded",
                                             "output": self.info(True, 10, sha)}
        self.journal.accept("inv-write")
        self.journal.outcomes["inv-write"] = {"state": "succeeded",
                                              "output": {"text": "Escrito «x»"}}
        with mock.patch.object(self.brain, "ask_confirmation",
                               return_value="allow_once") as ask:
            self.brain.execute_tool(
                "hefesto_write_file",
                {"path": "/Users/x/Documents/n.txt", "content": "nuevo", "mode": "overwrite"},
                JOB)
        self.assertEqual(
            ask.call_args.kwargs["summary"],
            (f"Sobrescribir «/Users/x/Documents/n.txt» (actual 10 B, sha256 {'b' * 12}… → nuevo 5 B)", "write"))
        self.assertEqual(self.journal.submits[1]["arguments"]["expectedHash"], sha)

    def test_hash_obsoleto_vuelve_como_texto(self):
        self.journal.accept("inv-info")
        self.journal.outcomes["inv-info"] = {"state": "succeeded",
                                             "output": self.info(True, 10, "c" * 64)}
        self.journal.accept("inv-write")
        self.journal.outcomes["inv-write"] = {"state": "succeeded",
                                              "output": {"text": "el archivo cambió desde la confirmación"}}
        with mock.patch.object(self.brain, "ask_confirmation", return_value="allow_once"):
            result = self.brain.execute_tool(
                "hefesto_write_file",
                {"path": "/Users/x/Documents/n.txt", "content": "x", "mode": "overwrite"},
                JOB)
        self.assertEqual(result, "el archivo cambió desde la confirmación")

    def test_renombrar_valida_destino_y_pide_tarjeta(self):
        self.journal.accept("inv-info")
        self.journal.outcomes["inv-info"] = {"state": "succeeded",
                                             "output": self.info(True, 4, "d" * 64)}
        self.journal.accept("inv-info-2")
        self.journal.outcomes["inv-info-2"] = {"state": "succeeded",
                                               "output": self.info(False)}
        self.journal.accept("inv-rn")
        self.journal.outcomes["inv-rn"] = {"state": "succeeded",
                                            "output": {"text": "Movido «a» → «b» (4 B)"}}
        with mock.patch.object(self.brain, "ask_confirmation",
                               return_value="allow_once") as ask:
            result = self.brain.execute_tool(
                "hefesto_rename",
                {"from": "/Users/x/Documents/a.txt", "to": "/Users/x/Documents/b.txt"},
                JOB)
        self.assertEqual(result, "Movido «a» → «b» (4 B)")
        self.assertIn("Mover «/Users/x/Documents/a.txt» → «/Users/x/Documents/b.txt»",
                      ask.call_args.kwargs["summary"][0])
        self.assertEqual(self.journal.submits[-1]["toolId"], "hefesto.rename")

    def test_renombrar_con_destino_existente_no_abre_tarjeta(self):
        self.journal.accept("inv-info")
        self.journal.outcomes["inv-info"] = {"state": "succeeded",
                                             "output": self.info(True, 4, "e" * 64)}
        self.journal.accept("inv-info-2")
        self.journal.outcomes["inv-info-2"] = {"state": "succeeded",
                                               "output": self.info(True, 2, "f" * 64)}
        with mock.patch.object(self.brain, "ask_confirmation") as ask:
            result = self.brain.execute_tool(
                "hefesto_rename",
                {"from": "/Users/x/Documents/a.txt", "to": "/Users/x/Documents/b.txt"},
                JOB)
        self.assertEqual(result, "el destino ya existe")
        ask.assert_not_called()
```

Además, en `test_hefesto_delegation.py`:
- cambia a `3` todos los literales `"protocolVersion": 2` de `FakeJournal` y `ControlJournal` (respuestas del journal falso);
- en los tests existentes de lectura sustituye `{"state": "succeeded", "outputText": "..."}` por `{"state": "succeeded", "output": {"text": "..."}}`;
- en `TrackerTests` sustituye `"outputText": None` por `"output": None` en los `outcomes`.

- [ ] **Step 2: ejecutar y ver el fallo**

Run: `cd server && python3 -m unittest test_hefesto_delegation.WriteFlowTests -v`
Expected: FAIL (`herramienta desconocida`).

- [ ] **Step 3: implementar**

Catálogo (junto a las otras tools):

```python
HEFESTO_WRITE_TOOL = {
    "id": "hefesto_write_file",
    "risk": "write",
    "requiresConfirmation": True,
    # El plan (tamaño y hash actuales) se pide dentro de _write_file antes de
    # la tarjeta; _handle_tool_call no debe abrir otra confirmación genérica.
    "planConfirmation": True,
    "description": ("Crea o sobrescribe un archivo de texto en el Mac con Hefesto. Usa "
                    "una ruta absoluta dentro de las raíces autorizadas (Documents, "
                    "Desktop, Downloads, Pictures, Music, Movies). mode=create falla si "
                    "el archivo existe; mode=overwrite lo reemplaza. Máximo 16 KiB de "
                    "texto. Cada escritura pide confirmación."),
    "schema": {
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "ruta absoluta del archivo"},
            "content": {"type": "string", "description": "texto a escribir (máx. 16 KiB)"},
            "mode": {"type": "string", "enum": ["create", "overwrite"],
                     "description": "create falla si existe; overwrite reemplaza"},
        },
        "required": ["path", "content", "mode"],
    },
}

HEFESTO_RENAME_TOOL = {
    "id": "hefesto_rename",
    "risk": "write",
    "requiresConfirmation": True,
    "planConfirmation": True,
    "description": ("Mueve o renombra un archivo en el Mac con Hefesto, dentro de las "
                    "raíces autorizadas. El destino no puede existir. Cada movimiento "
                    "pide confirmación."),
    "schema": {
        "type": "object",
        "properties": {
            "from": {"type": "string", "description": "ruta absoluta actual"},
            "to": {"type": "string", "description": "ruta absoluta nueva"},
        },
        "required": ["from", "to"],
    },
}

HEFESTO_TOOLS = [HEFESTO_TOOL, HEFESTO_LIST_TOOL, HEFESTO_READ_TOOL,
                 HEFESTO_WRITE_TOOL, HEFESTO_RENAME_TOOL]
```

En `_handle_tool_call`, la confirmación genérica se salta cuando la herramienta trae
su propio plan:

```python
        if tool["requiresConfirmation"] and not tool.get("planConfirmation"):
```

Routing en `execute_tool`:

```python
        if tool_id == "hefesto_write_file":
            return self._write_file(job, args)
        if tool_id == "hefesto_rename":
            return self._rename_file(job, args)
```

Sustituye `_delegate_read` por helpers compartidos:

```python
    def _hefesto_submit(self, tool_id: str, job: dict, arguments: dict) -> tuple[str | None, str | None]:
        journal, device_id = self._hefesto_journal, self._hefesto_device
        if journal is None or device_id is None:
            return None, "Hefesto no está disponible (desactivado)"
        with self.lock:
            if job["request_id"] in self.cancelled:
                return None, "cancelada a mitad de turno"
        submission = {
            "protocolVersion": 3,
            "submissionId": new_id("sub"),
            "requestId": job["request_id"],
            "sessionId": job["session_id"],
            "taskId": new_id("tsk"),
            "toolId": tool_id,
            "arguments": arguments,
        }
        status, response = journal.dispatch(
            hefesto.Peer("controller", device_id), "POST",
            "/v1/hefesto/submit", json.dumps(submission).encode("utf-8"))
        if status != 200:
            dlog(self, "HEFESTO",
                 f"submit rejected tool={tool_id} error={response.get('error')} request={job['request_id']}")
            return None, f"Hefesto no está disponible ({response.get('error', 'desconocido')})"
        return response["invocationId"], None

    def _hefesto_wait(self, invocation_id: str, job: dict, timeout: float) -> dict | str:
        journal, device_id = self._hefesto_journal, self._hefesto_device
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self.lock:
                if job["request_id"] in self.cancelled:
                    return "cancelada a mitad de turno"
            status, outcome = journal.dispatch(
                hefesto.Peer("controller", device_id), "POST",
                "/v1/hefesto/outcome",
                json.dumps({"protocolVersion": 3,
                            "invocationId": invocation_id}).encode("utf-8"))
            if status == 200 and outcome.get("state") in (
                    "succeeded", "expired", "cancelled", "unknown"):
                if outcome["state"] == "succeeded":
                    return outcome.get("output") or {}
                if outcome["state"] == "expired":
                    return "Hefesto no respondió a tiempo y la operación caducó."
                return "La operación se canceló."
            time.sleep(0.5)
        return "Hefesto no respondió a tiempo (timeout de la operación)"

    def _delegate(self, tool_id: str, job: dict, arguments: dict) -> str:
        invocation_id, error = self._hefesto_submit(tool_id, job, arguments)
        if error is not None:
            return error
        output = self._hefesto_wait(invocation_id, job, 15.0)
        if isinstance(output, str):
            return output
        return str(output.get("text") or "")

    def _delegate_read(self, tool_id: str, job: dict, args: dict) -> str:
        path = str(args.get("path", ""))
        if not path.startswith("/") or "\x00" in path or len(path.encode("utf-8")) > 1024:
            return "ruta inválida (debe ser una ruta absoluta dentro de las raíces autorizadas)"
        return self._delegate(tool_id, job, {"path": path})

    def _file_info(self, path: str, job: dict) -> dict:
        """Información del archivo para el plan; `{"error": texto}` si falla."""
        invocation_id, error = self._hefesto_submit("hefesto.file_info", job, {"path": path})
        if error is not None:
            return {"error": error}
        output = self._hefesto_wait(invocation_id, job, 5.0)
        if isinstance(output, str):
            return {"error": output}
        if "text" in output:
            # file_info responde con texto cuando rechaza la ruta o el archivo.
            return {"error": str(output["text"])}
        return output
```

El tracker asíncrono deja de leer `outputText` y extrae el texto de la salida tipada:

```python
            if response.get("state") in ("succeeded", "expired", "cancelled", "unknown"):
                output = response.get("output") or {}
                self._hefesto_finish(entry, response["state"], output.get("text"))
```

`_write_file` y `_rename_file`:

```python
    def _write_file(self, job: dict, args: dict) -> str:
        path = args.get("path")
        content = args.get("content")
        mode = args.get("mode")
        if (not isinstance(path, str) or not path.startswith("/") or "\x00" in path
                or len(path.encode("utf-8")) > 1024):
            return "ruta inválida (debe ser una ruta absoluta dentro de las raíces autorizadas)"
        if not isinstance(content, str) or "\x00" in content or len(content.encode("utf-8")) > 16 * 1024:
            return "contenido inválido (texto UTF-8 de hasta 16 KiB)"
        if mode not in ("create", "overwrite"):
            return "mode inválido (create u overwrite)"
        info = self._file_info(path, job)
        if "error" in info:
            return str(info["error"])
        size = len(content.encode("utf-8"))
        if mode == "create":
            if info["exists"]:
                return "ya existe; usa mode overwrite para reemplazarlo"
            summary = (f"Crear «{path}» ({size} B)", "write")
            expected = None
        else:
            if not info["exists"]:
                return "el archivo ya no existe; usa mode create"
            expected = info["sha256"]
            summary = (f"Sobrescribir «{path}» (actual {info['size']} B, "
                       f"sha256 {expected[:12]}… → nuevo {size} B)", "write")
        # El tiempo del usuario no consume el plazo de la operación.
        job["deadline"] = time.monotonic() + CONFIRM_TIMEOUT_S + 60
        if self.ask_confirmation(job["request_id"], job["session_id"],
                                 "hefesto_write_file", args,
                                 chat_id=job.get("chat_id"), summary=summary) != "allow_once":
            return "denegada por el usuario"
        return self._delegate("hefesto.write_file", job, {
            "path": path, "content": content, "expectedHash": expected})

    def _rename_file(self, job: dict, args: dict) -> str:
        source, target = args.get("from"), args.get("to")
        for value in (source, target):
            if (not isinstance(value, str) or not value.startswith("/") or "\x00" in value
                    or len(value.encode("utf-8")) > 1024):
                return "ruta inválida (debe ser una ruta absoluta dentro de las raíces autorizadas)"
        info = self._file_info(source, job)
        if "error" in info:
            return str(info["error"])
        if not info["exists"]:
            return "el origen no existe"
        destination = self._file_info(target, job)
        if "error" in destination:
            return str(destination["error"])
        if destination["exists"]:
            return "el destino ya existe"
        summary = (f"Mover «{source}» → «{target}» ({info['size']} B, "
                   f"sha256 {info['sha256'][:12]}…)", "write")
        job["deadline"] = time.monotonic() + CONFIRM_TIMEOUT_S + 60
        if self.ask_confirmation(job["request_id"], job["session_id"],
                                 "hefesto_rename", args,
                                 chat_id=job.get("chat_id"), summary=summary) != "allow_once":
            return "denegada por el usuario"
        return self._delegate("hefesto.rename", job, {"from": source, "to": target})
```

`ask_confirmation` con resumen ya calculado:

```python
    def ask_confirmation(self, request_id, session_id, tool_id, args,
                         chat_id=None, summary=None):
        if summary is None:
            summary = self._confirm_summary(tool_id, args)
        text, risk = summary
        ...
```

(y usa `text`/`risk` en el evento en lugar de recalcular).

`_confirm_summary` gana casos por si se llama sin plan:

```python
        if tool_id == "hefesto_write_file":
            return (f"Escribir «{args.get('path', '?')}»", "write")
        if tool_id == "hefesto_rename":
            return (f"Mover «{args.get('from', '?')}» → «{args.get('to', '?')}»", "write")
```

- [ ] **Step 4: ejecutar la suite Python completa**

Run: `cd server && python3 -m unittest discover -p "test_*.py"`
Expected: PASS (nuevos WriteFlowTests y los ReadFlowTests actualizados a `output`).

- [ ] **Step 5: commit**

```bash
git add server/atlas_relay.py server/test_hefesto_delegation.py
git commit -m "feat(hefesto): escritura con plan y confirmación desde el relay"
```

---

### Task 6: contratos y documentación

**Files:**
- Modify: `docs/contracts.md`
- Modify: `docs/hefesto-plan.md` (§5 y §6)
- Modify: `docs/superpowers/specs/2026-10-30-hefesto-escritura-design.md` (solo si el código se desvió del spec; si no, no tocar)

**Interfaces:**
- Consumes: todo lo anterior.
- Produces: documentación pública v3.

- [ ] **Step 1: actualizar `docs/contracts.md`**

En la sección «Hefesto v2 — eco y lectura»:
- Título a «Hefesto v3 — eco, lectura y escritura».
- `protocolVersion: 2` → `3` en todas las menciones.
- Tabla de rutas: añade `POST /v1/hefesto/submit` con los toolIds nuevos y `output` tipado.
- Nueva subsección «Escritura»: `write_file` (`{path, content, expectedHash}`), `rename` (`{from, to}`), `file_info` interna (`{path}` → `{exists, size, sha256}` o `{text}`); límites 16 KiB/16 384 B; contenido nunca en sqlite; tarjetas con hash corto; errores estables (`ya existe`, `el archivo cambió desde la confirmación`, `el destino ya existe`, `fuera de las raíces autorizadas`, `no se puede mover entre volúmenes`).

- [ ] **Step 2: actualizar `docs/hefesto-plan.md`**

- §5: título «Protocolo v3 (eco, lectura y escritura)», versión 3, tabla de toolIds con las tres nuevas filas y la nota de `expectedHash` inyectado por el relay.
- §6, puerta 4: marcar el corte 2 como cerrado con la fecha de entrega y lo que queda fuera (borrado).
- §8: entrada nueva al final con commits, pruebas y prueba en vivo (se completará en Task 7).

- [ ] **Step 3: verificar consistencia**

Run: `rg -n "protocolVersion: 2|protocolVersion\": 2" docs/contracts.md docs/hefesto-plan.md`
Expected: sin resultados (todo v3).

- [ ] **Step 4: commit**

```bash
git add docs/contracts.md docs/hefesto-plan.md
git commit -m "docs(hefesto): contratos y plan del protocolo v3 de escritura"
```

---

### Task 7: verificación global, despliegue y prueba en vivo

**Files:**
- Modify: `docs/implemented.md`, `docs/implementation-plan.md`, `docs/hefesto-plan.md` §8

**Interfaces:**
- Consumes: todo.
- Produces: entrega cerrada, desplegada y registrada.

- [ ] **Step 1: verificación global**

Run:
```bash
cargo test --workspace
cargo clippy -p atlas-mac --all-targets -- -D warnings
cargo fmt -p atlas-mac -p atlas-next -- --check
cd server && python3 -m unittest discover -p "test_*.py"
cd .. && npm test && npm run build
```
Expected: todo PASS; lint del frontend sin errores nuevos (los 3 de Hub Panel son previos).

- [ ] **Step 2: desplegar el relay (requiere permiso)**

Run: `./scripts/deploy-relay.sh`
Expected: `/v1/health` OK y servicio activo.

- [ ] **Step 3: bundle de la app (requiere permiso)**

Run:
```bash
APPLE_SIGNING_IDENTITY="Apple Development: adria_bgcma@hotmail.com (Y329B5UZ34)" npm run tauri build
codesign --verify --deep --strict --verbose=2 "target/release/bundle/macos/Atlas.app"
ditto "target/release/bundle/macos/Atlas.app" "/Applications/Atlas.app"
```
Expected: firma válida e instalación correcta (el paso DMG puede fallar sin afectar al `.app`).

- [ ] **Step 4: prueba en vivo (con el usuario)**

1. Relanzar la app con `ATLAS_HEFESTO_*` y comprobar registro `online`.
2. En el chat: «Crea /Users/adria/Documents/atlas-nota-hefesto.txt con el texto hola mundo» → tarjeta «Crear … (11 B)» → permitir → archivo creado con `hola mundo`.
3. «Sobrescribe esa nota con: adiós» → tarjeta con el sha256 actual → permitir → contenido reemplazado.
4. Hash obsoleto: pedir otra sobrescritura, y antes de aprobar modificar el archivo a mano; al aprobar debe responder «el archivo cambió desde la confirmación» y el archivo conserva el contenido manual.
5. «Mueve la nota a atlas-nota-hefesto-2.txt» → tarjeta → permitir → movido; el destino existente se rechaza.
6. «Escribe en /etc/atlas.txt» → rechazo «fuera de las raíces autorizadas» sin tarjeta.
7. Comprobar en el journal que el contenido no quedó en sqlite: `sqlite3 .../hefesto.sqlite "SELECT submission FROM invocations ORDER BY rowid DESC LIMIT 3"` no muestra el texto.

- [ ] **Step 5: registrar y commitear**

- `docs/implemented.md`: entrada «Escritura segura de archivos de Hefesto (puerta 4, corte 2)» con commits de las Tasks 1–6 y resultado de la prueba en vivo.
- `docs/implementation-plan.md`: marcar la puerta 4 completa y anotar el corte.
- `docs/hefesto-plan.md` §8: completar la entrada con la prueba en vivo y la auditoría de procesos.

```bash
git add docs/implemented.md docs/implementation-plan.md docs/hefesto-plan.md
git commit -m "docs(hefesto): registra la escritura segura y su prueba en vivo"
```

- [ ] **Step 6: auditoría de procesos**

Run: `pgrep -fl "atlas-next|vite|tauri|hefesto" | grep -v pgrep; lsof -nP -iTCP -sTCP:LISTEN | grep -E ":(5173|5174|8765|8767)" || true; launchctl list | grep -i atlas || true`
Expected: solo la app del usuario; sin Vite ni pruebas Python; puertos de dev libres; sin agentes nuevos.
