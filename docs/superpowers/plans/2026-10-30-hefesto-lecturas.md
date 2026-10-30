# Lectura segura de archivos de Hefesto — Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Que el LLM pueda listar directorios y leer archivos de texto dentro de raíces autorizadas del Mac, con consentimiento por lectura y resultado síncrono al modelo.

**Architecture:** Protocolo Hefesto sube a v2 con un `toolId` ampliado (`hefesto.echo`, `hefesto.list_dir`, `hefesto.read_file`) y `arguments` por herramienta (`{text}` o `{path}`). El dispositivo (Rust) gana un despachador de herramientas con resolución de raíces seguras; el relay (Python) añade el catálogo, la confirmación y un bucle síncrono que sondea `outcome` hasta obtener el contenido.

**Tech Stack:** Rust (Tauri, `crates/atlas-mac`), Python 3.11+ (relay/journal, solo stdlib), JSON camelCase v2, unittest.

**Spec:** `docs/superpowers/specs/2026-10-30-hefesto-lecturas-design.md` — el plan argumenta desde el spec; los ejecutores leen ambos.

## Global Constraints

- **Permisos:** no commitear sin permiso explícito del usuario; no push/reescribir historial/cambiar rama sin permiso (AGENTS.md).
- **Versión:** `protocolVersion == 2` en todo el protocolo Hefesto (journal, peticiones del controller en el relay, wire de Rust). `1` se rechaza con `invalid_version`.
- **Regla de espejo:** todo cambio de contrato se refleja en `server/hefesto.py`, `crates/atlas-mac/src/wire.rs`, `docs/hefesto-plan.md` §5 y `docs/contracts.md`.
- **Constantes (verbatim del spec):** `READ_MAX_BYTES = 16 * 1024`, `LIST_MAX_ENTRIES = 100`, `PATH_MAX_BYTES = 1024`, `MAX_OUTPUT_BYTES = 64 * 1024`, `READ_WAIT_TIMEOUT_S = 15`, `OUTCOME_POLL_EVERY = 0.5`.
- **Solo lectura:** nada de escritura; consentimiento por lectura; resultado síncrono; sin mensaje «Hefesto:» aparte para lecturas.
- **Procesos:** sin procesos huérfanos en el Mac; los servidores de dev se matan al terminar la prueba.
- **Relay:** corre en `atlas-hub`; se despliega solo con `./scripts/deploy-relay.sh`.
- **Verificación:** `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`, `cargo test -p atlas-mac`, `cargo test --workspace`, `cargo check --workspace`.

---

### Task 1: Bump de protocolo a v2 (Rust + Python + fixtures)

Sube el número de versión del protocolo a 2 sin cambiar comportamiento: el eco sigue
funcionando igual, solo cambia `protocolVersion`. Es un cambio coordinado en los dos
lados (Rust y Python) para que el test de integración Rust–Python (`cargo test -p
atlas-mac`) siga verde en cada commit.

**Files:**
- Modify: `crates/atlas-mac/src/wire.rs:45-64` (tipo `ProtocolVersion`)
- Modify: `crates/atlas-mac/src/client.rs:222-238` (cuerpos de error en tests)
- Modify: `server/hefesto.py:58,119,263,270,337,344,353,359,380` (literales `protocolVersion`)
- Modify: `server/atlas_relay.py:836,959,984` (peticiones del controller al journal)
- Modify: `server/test_hefesto.py` (helper `submission` y aserciones)
- Modify: `server/test_hefesto_delegation.py` (fakes)
- Modify: `server/test_hefesto_integration.py:217`

**Interfaces:**
- Consumes: nada nuevo.
- Produces: protocolo v2 en ambos extremos; nada más.

- [ ] **Step 1: Cambiar la constante de versión en Rust**

En `crates/atlas-mac/src/wire.rs`, dentro de `TryFrom<u64> for ProtocolVersion` (línea ~52) cambia la validación:

```rust
impl TryFrom<u64> for ProtocolVersion {
    type Error = Error;
    fn try_from(value: u64) -> Result<Self> {
        if value == 2 {
            Ok(Self)
        } else {
            Err(Error::InvalidVersion)
        }
    }
}
```

Y en `From<ProtocolVersion> for u64` (línea ~61):

```rust
impl From<ProtocolVersion> for u64 {
    fn from(_: ProtocolVersion) -> Self {
        2
    }
}
```

- [ ] **Step 2: Cambiar los literales `protocolVersion` en `server/hefesto.py`**

Sustituye **todas** las apariciones de `"protocolVersion": 1` por `"protocolVersion": 2`
(líneas 58, 263, 270, 337, 344, 353, 359, 380) y el chequeo de versión (línea 119):

```python
if type(body["protocolVersion"]) is not int or body["protocolVersion"] != 2:
    raise _Rejected("invalid_version")
```

- [ ] **Step 3: Cambiar las peticiones del controller en `server/atlas_relay.py`**

En las tres peticiones que el relay hace al journal como controller, cambia el literal a 2:
- Línea 836 (`_delegate_echo`, submit del eco): `"protocolVersion": 1,` → `"protocolVersion": 2,`
- Línea 959 (`hefesto_suspend`, control): `json.dumps({"protocolVersion": 1, "action": "abort", ...})` → `json.dumps({"protocolVersion": 2, ...})`
- Línea 984 (`_hefesto_poll`, outcome): `json.dumps({"protocolVersion": 1, "invocationId": ...})` → `json.dumps({"protocolVersion": 2, ...})`

- [ ] **Step 4: Actualizar los fixtures y aserciones de Python**

En `server/test_hefesto.py`:
- Helper `encoded` (línea ~35): `{"protocolVersion": 1, **fields}` → `{"protocolVersion": 2, **fields}`.
- En la clase `JournalTests`, los métodos `ok` y `error` asertan la versión: `self.assertEqual(value["protocolVersion"], 1)` (línea ~73) → `2`, y `self.assertEqual(value, {"protocolVersion": 1, "error": code})` (línea ~79) → `2`.
- Todas las aserciones `{"protocolVersion": 1, ...}` → `{"protocolVersion": 2, ...}` (líneas 102, 115, 163, 247, 534-541, 562, 579, 864, 877, 886). Los literales de `protocolVersion=version` en las pruebas de versión inválida (línea ~496) ya pasan el valor como parámetro: verifica que la prueba de `invalid_version` usa un valor distinto de 2 (p.ej. `1` o `3`) y sigue esperando `invalid_version`.

En `server/test_hefesto_delegation.py`: cambia `"protocolVersion": 1` → `"protocolVersion": 2` en los fakes (líneas 30-41, 67-79, 95).

En `server/test_hefesto_integration.py:217`: `body='{"protocolVersion":1}'` → `body='{"protocolVersion":2}'`.

- [ ] **Step 5: Actualizar los cuerpos de error en tests de Rust**

En `crates/atlas-mac/src/client.rs`, test `bounded_and_sanitized_responses` (líneas ~222-231): cambia `"protocolVersion":1` → `"protocolVersion":2` en los tres cuerpos JSON de ejemplo.

- [ ] **Step 6: Ejecutar las suites y verificar**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: `OK` (93 pruebas, las mismas que antes).

Run: `cargo test -p atlas-mac`
Expected: todos los tests verdes (incluida la integración Rust–Python con certificados efímeros).

Run: `cargo test --workspace` y `cargo check --workspace`
Expected: verdes.

- [ ] **Step 7: Commit**

```bash
git add crates/atlas-mac/src/wire.rs crates/atlas-mac/src/client.rs server/hefesto.py server/atlas_relay.py server/test_hefesto.py server/test_hefesto_delegation.py server/test_hefesto_integration.py
git commit -m "feat(hefesto): protocolo v2 (solo versión, sin cambio de comportamiento)"
```

---

### Task 2: Módulo `tools.rs` de lectura (Rust)

Crea el módulo de herramientas de solo lectura: raíces seguras con contención real y
las funciones `list_dir`/`read_file` que devuelven texto (los errores van como texto,
según spec §3.4). Totalmente testeado con directorios temporales; aún no se cablea.

**Files:**
- Create: `crates/atlas-mac/src/tools.rs`
- Modify: `crates/atlas-mac/src/lib.rs` (declarar módulo + variante `Error::InvalidPath`)

**Interfaces:**
- Consumes: `crate::{Error, Result}`.
- Produces:
  - `pub struct Roots { entries: Vec<PathBuf> }` con `Roots::new(Vec<PathBuf>) -> Roots`, `Roots::macos_default() -> Result<Roots>`, `Roots::resolve(&self, path: &str) -> Option<PathBuf>`.
  - `pub fn list_dir(roots: &Roots, path: &str) -> String`
  - `pub fn read_file(roots: &Roots, path: &str) -> String`
  - `pub const READ_MAX_BYTES: u64 = 16 * 1024;` y `pub const LIST_MAX_ENTRIES: usize = 100;`

- [ ] **Step 1: Añadir la variante `InvalidPath` al enum `Error`**

En `crates/atlas-mac/src/lib.rs`, añade al enum `Error` (junto a `InvalidText`) la variante `InvalidPath`, y en el `match` de `Error::code` su código:

```rust
    InvalidPath,
```

```rust
            Self::InvalidPath => "invalid_path",
```

- [ ] **Step 2: Escribir el módulo `tools.rs`**

```rust
//! Herramientas de solo lectura: raíces seguras y listado/lectura de archivos.

use crate::{Error, Result};
use std::path::{Path, PathBuf};

pub const READ_MAX_BYTES: u64 = 16 * 1024;
pub const LIST_MAX_ENTRIES: usize = 100;

#[derive(Clone, Debug)]
pub struct Roots {
    entries: Vec<PathBuf>,
}

impl Roots {
    pub fn new(entries: Vec<PathBuf>) -> Self {
        let entries = entries
            .iter()
            .filter_map(|p| p.canonicalize().ok())
            .collect();
        Self { entries }
    }

    pub fn macos_default() -> Result<Self> {
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or(Error::InvalidPath)?;
        let names = ["Documents", "Desktop", "Downloads", "Pictures", "Music", "Movies"];
        let mut entries = Vec::new();
        for name in names {
            let dir = home.join(name);
            if dir.is_dir() {
                entries.push(dir.canonicalize().unwrap_or(dir));
            }
        }
        Ok(Self { entries })
    }

    pub fn resolve(&self, path: &str) -> Option<PathBuf> {
        let candidate = Path::new(path);
        if !candidate.is_absolute() {
            return None;
        }
        let resolved = candidate.canonicalize().ok()?;
        if self.entries.iter().any(|root| resolved.starts_with(root)) {
            Some(resolved)
        } else {
            None
        }
    }
}

pub fn list_dir(roots: &Roots, path: &str) -> String {
    let Some(dir) = roots.resolve(path) else {
        return "fuera de las raíces autorizadas".to_string();
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return "no se pudo listar".to_string();
    };
    let mut entries: Vec<_> = read.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    if entries.is_empty() {
        return "directorio vacío".to_string();
    }
    let mut lines = Vec::new();
    for entry in entries.iter().take(LIST_MAX_ENTRIES) {
        let name = entry.file_name().to_string_lossy().to_string();
        match entry.metadata() {
            Ok(m) if m.is_dir() => lines.push(format!("{name}\tdirectorio")),
            Ok(m) => lines.push(format!("{name}\tarchivo\t{}", m.len())),
            Err(_) => lines.push(format!("{name}\tdesconocido")),
        }
    }
    lines.join("\n")
}

pub fn read_file(roots: &Roots, path: &str) -> String {
    let Some(file) = roots.resolve(path) else {
        return "fuera de las raíces autorizadas".to_string();
    };
    let Ok(meta) = std::fs::metadata(&file) else {
        return "no se pudo leer".to_string();
    };
    if !meta.is_file() {
        return "no es un archivo regular".to_string();
    }
    if meta.len() > READ_MAX_BYTES {
        return format!("demasiado grande: {} bytes (máximo {})", meta.len(), READ_MAX_BYTES);
    }
    let Ok(bytes) = std::fs::read(&file) else {
        return "no se pudo leer".to_string();
    };
    if bytes.contains(&0) {
        return "binario/no legible".to_string();
    }
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => "binario/no legible".to_string(),
    }
}
```

- [ ] **Step 3: Declarar el módulo**

En `crates/atlas-mac/src/lib.rs`, añade `pub mod tools;` junto a los otros módulos.

- [ ] **Step 4: Escribir los tests del módulo**

Añade al final de `crates/atlas-mac/src/tools.rs` un módulo `tests` con un helper de
setup que crea un directorio temporal único y lo limpia al terminar:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "atlas-tools-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
```

Casos de prueba (cada uno con su setup de directorios temporales):

```rust
    #[test]
    fn list_dir_enumera_nombres_ordenados_dentro_de_la_raiz() {
        let dir = TempDir::new("list");
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        std::fs::create_dir(dir.path().join("b")).unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        let out = list_dir(&roots, dir.path().to_str().unwrap());
        assert!(out.contains("a.txt"));
        assert!(out.contains("directorio"));
    }

    #[test]
    fn read_file_devuelve_contenido_utf8() {
        let dir = TempDir::new("read");
        std::fs::write(dir.path().join("f.txt"), "hola mundo").unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        assert_eq!(
            read_file(&roots, dir.path().join("f.txt").to_str().unwrap()),
            "hola mundo"
        );
    }

    #[test]
    fn ruta_fuera_de_las_raices_se_rechaza() {
        let dir = TempDir::new("in");
        let outside = TempDir::new("out");
        std::fs::write(outside.path().join("s.txt"), "secreto").unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        assert_eq!(
            read_file(&roots, outside.path().join("s.txt").to_str().unwrap()),
            "fuera de las raíces autorizadas"
        );
    }

    #[test]
    fn enlace_que_escapa_de_la_raiz_se_rechaza() {
        let dir = TempDir::new("link");
        let outside = TempDir::new("linkout");
        std::fs::write(outside.path().join("s.txt"), "secreto").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("s.txt"),
            dir.path().join("link.txt"),
        )
        .unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        assert_eq!(
            read_file(&roots, dir.path().join("link.txt").to_str().unwrap()),
            "fuera de las raíces autorizadas"
        );
    }

    #[test]
    fn archivo_demasiado_grande_se_rechaza_con_tamano() {
        let dir = TempDir::new("big");
        let big = dir.path().join("big.txt");
        std::fs::write(&big, vec![b'a'; (READ_MAX_BYTES + 1) as usize]).unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        let out = read_file(&roots, big.to_str().unwrap());
        assert!(out.starts_with("demasiado grande"));
    }

    #[test]
    fn binario_con_nul_se_rechaza() {
        let dir = TempDir::new("bin");
        let bin = dir.path().join("b.bin");
        std::fs::write(&bin, [0u8, 1, 2, 3]).unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        assert_eq!(
            read_file(&roots, bin.to_str().unwrap()),
            "binario/no legible"
        );
    }

    #[test]
    fn ruta_relativa_se_rechaza() {
        let dir = TempDir::new("rel");
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        assert_eq!(read_file(&roots, "f.txt"), "fuera de las raíces autorizadas");
    }
```

- [ ] **Step 5: Ejecutar los tests y verificar**

Run: `cargo test -p atlas-mac tools::`
Expected: los 7 tests nuevos verdes; el resto de la suite sigue verde.

- [ ] **Step 6: Commit**

```bash
git add crates/atlas-mac/src/tools.rs crates/atlas-mac/src/lib.rs
git commit -m "feat(hefesto): herramientas de lectura con raíces seguras"
```

---

### Task 3: Journal v2 — validación por herramienta y reconstrucción de `next`

Amplía el journal para aceptar `hefesto.list_dir` y `hefesto.read_file`: validación de
`arguments` por herramienta, `output` acotado a 64 KiB, y reconstrucción de
`toolId`/`arguments` en `next` desde el `submission` guardado (sin migración de esquema).

**Files:**
- Modify: `server/hefesto.py:129-142` (validación de arguments/output), `335` (texto guardado), `353-358` (reconstrucción de `next`), `360-379` (chequeo de resultado)
- Modify: `server/test_hefesto.py` (tests nuevos)

**Interfaces:**
- Consumes: protocolo v2 (Task 1).
- Produces: el journal acepta `toolId ∈ {hefesto.echo, hefesto.list_dir, hefesto.read_file}`; `next` devuelve `{toolId, arguments}` fieles al submit; `result` valida el eco contra el `arguments` original y deja libre el texto de list/read.

- [ ] **Step 1: Sustituir la validación de arguments/output en `_validate`**

Reemplaza el bloque actual (líneas 129-141, desde `if operation == "submit" and body["toolId"] != "hefesto.echo":` hasta el final del bucle `for key in ("arguments", "output")`) por:

```python
    if operation == "submit" and body["toolId"] not in ("hefesto.echo", "hefesto.list_dir", "hefesto.read_file"):
        raise _Rejected("unsupported_tool")
    if operation == "submit":
        args = body["arguments"]
        if body["toolId"] == "hefesto.echo":
            if type(args) is not dict or set(args) != {"text"} or type(args["text"]) is not str:
                raise _Rejected("invalid_text")
            if len(args["text"].encode("utf-8")) > 256:
                raise _Rejected("invalid_text")
        else:
            if type(args) is not dict or set(args) != {"path"} or type(args["path"]) is not str:
                raise _Rejected("invalid_path")
            path = args["path"]
            if not path.startswith("/") or "\x00" in path or len(path.encode("utf-8")) > 1024:
                raise _Rejected("invalid_path")
    if "output" in body:
        value = body["output"]
        if type(value) is not dict or set(value) != {"text"} or type(value["text"]) is not str:
            raise _Rejected("invalid_text")
        if len(value["text"].encode("utf-8")) > 64 * 1024:
            raise _Rejected("invalid_text")
```

- [ ] **Step 2: Guardar el texto del submit sin asumir `arguments.text`**

En `_operate`, operación `submit`, el INSERT guarda `data["arguments"]["text"]` (línea ~335). Cámbialo a:

```python
                              data["arguments"].get("text", ""), int(wall * 1000) + INVOCATION_SECONDS * 1000,
```

(la columna `text` queda como eco para el chequeo de resultado; list/read guardan `""`).

- [ ] **Step 3: Reconstruir `toolId` y `arguments` en `next`**

En `_operate`, operación `next`, dentro del `if active is not None:` (líneas ~350-358), añade al principio:

```python
            submitted = json.loads(active["submission"])
```

y sustituye las claves hardcodeadas del dict de `invocation`:

```python
                invocation = {"protocolVersion": 2, "invocationId": active["invocation_id"],
                              "requestId": active["request_id"], "sessionId": active["session_id"],
                              "taskId": active["task_id"], "deviceId": device_id,
                              "instanceId": active["instance_id"], "generation": active["generation"],
                              "toolId": submitted["toolId"], "arguments": submitted["arguments"],
                              "expiresAtUnixMs": active["expires_ms"]}
```

- [ ] **Step 4: Relajar el chequeo de resultado solo para el eco**

En `_operate`, operación `result`, antes del primer chequeo (línea ~368), añade:

```python
        submitted = json.loads(invocation["submission"])
        if submitted["toolId"] == "hefesto.echo" and data["output"]["text"] != invocation["text"]:
            raise _Rejected("result_conflict", 409)
```

y elimina la línea original `if data["output"]["text"] != invocation["text"]: raise _Rejected("result_conflict", 409)` (ya cubierta por la nueva, que solo se aplica al eco). El chequeo de repetición de `succeeded` (línea ~372) se mantiene igual.

- [ ] **Step 5: Escribir los tests del journal**

En `server/test_hefesto.py`, añade una clase que herede de `JournalTests` (así reutiliza
`self.ok`/`self.error`/`self.call`/`self.ready`/`self.next`/`submission(...)` ya definidos):

```python
class ReadToolTests(JournalTests):
    def test_submit_read_file_aceptado(self):
        self.ready()
        ack = self.ok("submit", CONTROLLER, **submission(
            toolId="hefesto.read_file", arguments={"path": "/Users/x/Documents/a.txt"}))
        self.assertEqual(set(ack), {"protocolVersion", "invocationId", "state"})
        self.assertEqual(ack["state"], "pending")

    def test_submit_list_dir_aceptado(self):
        self.ready()
        ack = self.ok("submit", CONTROLLER, **submission(
            toolId="hefesto.list_dir", arguments={"path": "/Users/x/Documents"}))
        self.assertEqual(ack["state"], "pending")

    def test_submit_con_tool_desconocido_rechazado(self):
        self.ready()
        self.error("unsupported_tool", "submit", CONTROLLER,
                   **submission(toolId="hefesto.mkdir", arguments={"path": "/x"}))

    def test_read_con_argumento_text_rechazado(self):
        self.ready()
        # toolId de lectura con arguments {text} (shape de eco) => invalid_path
        self.error("invalid_path", "submit", CONTROLLER,
                   **submission(toolId="hefesto.read_file"))

    def test_read_con_ruta_relativa_rechazada(self):
        self.ready()
        self.error("invalid_path", "submit", CONTROLLER,
                   **submission(toolId="hefesto.read_file", arguments={"path": "relativo"}))

    def test_next_devuelve_toolid_y_arguments_del_submit(self):
        self.ready()
        ack = self.ok("submit", CONTROLLER, **submission(
            toolId="hefesto.read_file", arguments={"path": "/Users/x/Documents/a.txt"}))
        invocation = self.next()
        self.assertEqual(invocation["invocationId"], ack["invocationId"])
        self.assertEqual(invocation["toolId"], "hefesto.read_file")
        self.assertEqual(invocation["arguments"], {"path": "/Users/x/Documents/a.txt"})

    def test_result_de_lectura_acepta_texto_libre(self):
        self.ready()
        self.ok("submit", CONTROLLER, **submission(
            toolId="hefesto.read_file", arguments={"path": "/Users/x/Documents/a.txt"}))
        invocation = self.next()
        done = self.ok("result", **self.result_fields(invocation, output={"text": "contenido leído"}))
        self.assertEqual(done["state"], "succeeded")

    def test_result_de_eco_sigue_exigiendo_eco_exacto(self):
        self.ready()
        self.send(text="hola")
        invocation = self.next()
        self.error("result_conflict", "result",
                   **self.result_fields(invocation, output={"text": "distinto"}))
```

- [ ] **Step 6: Ejecutar las pruebas y verificar**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: `OK` con las pruebas nuevas (93 + 8 nuevas).

- [ ] **Step 7: Commit**

```bash
git add server/hefesto.py server/test_hefesto.py
git commit -m "feat(hefesto): journal v2 con herramientas de lectura"
```

---

### Task 4: Wire v2 + despachador de herramientas en el dispositivo (Rust)

Generaliza el wire de Rust (nuevos `toolId`, `arguments` por herramienta, `output` de
texto) y convierte el ejecutor de eco en un despachador que ejecuta `list_dir`/`read_file`
vía `tools::run` y conserva el eco.

**Files:**
- Modify: `crates/atlas-mac/src/wire.rs` (constantes, `ToolId`, `ToolPath`, `ToolArguments`, `OutputPayload`, `TextOutput`, `SubmitRequest.arguments`, `Invocation.arguments`, `ResultRequest.output`)
- Modify: `crates/atlas-mac/src/executor.rs` (rename `EchoExecutor`→`Executor`, `roots`, dispatch)
- Modify: `crates/atlas-mac/src/runtime.rs` (`Executor`, `Runtime::new` con `roots`, badge en `poll`, tests)
- Modify: `crates/atlas-mac/src/lib.rs` (`pub use executor::Executor`, variante `Error::InvalidArguments`, re-export de `Roots`)

**Interfaces:**
- Consumes: `tools::{Roots, list_dir, read_file}` (Task 2); protocolo v2 (Task 1).
- Produces:
  - `enum ToolId { Echo, ListDir, ReadFile }` con renames `"hefesto.echo"`, `"hefesto.list_dir"`, `"hefesto.read_file"`.
  - `enum ToolArguments { Echo(EchoPayload), Path(PathPayload) }` (untagged).
  - `struct PathPayload { path: ToolPath }`; `struct ToolPath(String)` con `ToolPath::new`, `as_str`.
  - `struct OutputPayload { text: TextOutput }`; `struct TextOutput(String)` con `TextOutput::new`, `as_str`.
  - `pub struct Executor` (antes `EchoExecutor`) con `Executor::new(device_id, instance_id, roots: Roots)`.
  - `Runtime::new(transport, device_id, roots: Roots)`.

- [ ] **Step 1: Añadir la variante `InvalidArguments`**

En `crates/atlas-mac/src/lib.rs`, añade `InvalidArguments` al enum `Error` y `Self::InvalidArguments => "invalid_arguments"` en `code()`.

- [ ] **Step 2: Ampliar el wire**

En `crates/atlas-mac/src/wire.rs`:

1. Añade las constantes tras `MAX_ECHO_BYTES`:

```rust
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_PATH_BYTES: usize = 1024;
```

2. Amplía `ToolId` (líneas ~182-186):

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ToolId {
    #[serde(rename = "hefesto.echo")]
    Echo,
    #[serde(rename = "hefesto.list_dir")]
    ListDir,
    #[serde(rename = "hefesto.read_file")]
    ReadFile,
}
```

3. Añade los tipos nuevos tras `EchoPayload` (línea ~180):

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ToolPath(String);

impl ToolPath {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_PATH_BYTES
            || !value.starts_with('/')
            || value.contains('\0')
            || value.chars().any(char::is_control)
        {
            return Err(Error::InvalidPath);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ToolPath {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<ToolPath> for String {
    fn from(value: ToolPath) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathPayload {
    pub path: ToolPath,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolArguments {
    Echo(EchoPayload),
    Path(PathPayload),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TextOutput(String);

impl TextOutput {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() > MAX_OUTPUT_BYTES {
            return Err(Error::InvalidText);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TextOutput {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<TextOutput> for String {
    fn from(value: TextOutput) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputPayload {
    pub text: TextOutput,
}
```

4. Cambia los campos:
   - `SubmitRequest.arguments: EchoPayload` → `ToolArguments`.
   - `Invocation.arguments: EchoPayload` → `ToolArguments`.
   - `ResultRequest.output: EchoPayload` → `OutputPayload`.

- [ ] **Step 3: Renombrar y despachar en el ejecutor**

En `crates/atlas-mac/src/executor.rs`:

1. Renombra `EchoExecutor` → `Executor` (struct e `impl`), y `CompletedEcho` → `CompletedInvocation`. Actualiza `Executor::new` para aceptar `roots` y guardarlo:

```rust
pub struct Executor {
    device_id: OpaqueId,
    instance_id: OpaqueId,
    roots: crate::tools::Roots,
    // ... resto de campos sin cambios
}

impl Executor {
    pub fn new(device_id: OpaqueId, instance_id: OpaqueId, roots: crate::tools::Roots) -> Self {
        Self {
            device_id,
            instance_id,
            roots,
            // ... resto de campos inicializados como antes
        }
    }
```

2. En `execute`, sustituye la construcción del resultado (la línea `let result = ResultRequest { ... output: invocation.arguments.clone() ... }`) por el despacho:

```rust
        let text = match (&invocation.tool_id, &invocation.arguments) {
            (ToolId::Echo, ToolArguments::Echo(payload)) => payload.text.as_str().to_string(),
            (ToolId::ListDir, ToolArguments::Path(payload)) => {
                crate::tools::list_dir(&self.roots, payload.path.as_str())
            }
            (ToolId::ReadFile, ToolArguments::Path(payload)) => {
                crate::tools::read_file(&self.roots, payload.path.as_str())
            }
            _ => return Err(Error::InvalidArguments),
        };
        let output = OutputPayload {
            text: TextOutput::new(text)?,
        };
        let result = ResultRequest {
            protocol_version: ProtocolVersion,
            instance_id: self.instance_id.clone(),
            generation: invocation.generation,
            invocation_id: invocation.invocation_id.clone(),
            output,
        };
```

(el resto de `execute` — validación, idempotencia con `self.completed` — no cambia).

- [ ] **Step 4: Actualizar el runtime**

En `crates/atlas-mac/src/runtime.rs`:

1. `use crate::executor::{Clock, Executor};` y `struct Session { executor: Executor, ... }`.
2. `Runtime` gana un campo `roots: crate::tools::Roots`; `Runtime::new(transport, device_id, roots)` lo guarda.
3. En `spawn`, `let executor = Executor::new(self.device_id.clone(), instance.clone(), self.roots.clone());`.
4. En `Session::poll`, sustituye la línea final `Ok(Some(invocation.arguments.text.as_str().to_string()))` por el badge por herramienta:

```rust
        let badge = match &invocation.arguments {
            ToolArguments::Echo(payload) => Some(payload.text.as_str().to_string()),
            ToolArguments::Path(_) => None,
        };
        transport.result(&result).map_err(classify)?;
        Ok(badge)
```

5. Actualiza los tests: el helper `runtime()` pasa un `Roots` de test; el `Fake::next` construye `arguments: ToolArguments::Echo(EchoPayload::new(text).unwrap())`; la aserción `state.results[0].output.text.as_str()` sigue válida (ahora es `TextOutput::as_str`).

```rust
    fn runtime() -> Runtime<Fake> {
        Runtime::new(
            Fake::new(),
            OpaqueId::new(DEVICE).unwrap(),
            crate::tools::Roots::new(vec![std::env::temp_dir()]),
        )
    }
```

En `Fake::next`, cambia la construcción del `Invocation`:

```rust
                    arguments: ToolArguments::Echo(EchoPayload::new(text).unwrap()),
```

- [ ] **Step 5: Actualizar las re-exportaciones**

En `crates/atlas-mac/src/lib.rs`:
- `pub use executor::{Clock, EchoExecutor, RegistrationAttempt, ResumeAttempt};` → `pub use executor::{Clock, Executor, RegistrationAttempt, ResumeAttempt};`
- Añade `pub use tools::Roots;` (opcional, para que el cableado de la app lo use).

- [ ] **Step 6: Ejecutar las suites y verificar**

Run: `cargo test -p atlas-mac`
Expected: verdes (los tests de `runtime` actualizados + `tools` + integración Rust–Python).

Run: `cargo check --workspace`
Expected: sin errores.

- [ ] **Step 7: Commit**

```bash
git add crates/atlas-mac/src/wire.rs crates/atlas-mac/src/executor.rs crates/atlas-mac/src/runtime.rs crates/atlas-mac/src/lib.rs
git commit -m "feat(hefesto): despachador de herramientas en el dispositivo (lectura)"
```

---

### Task 5: Relay — catálogo, confirmación y lectura síncrona

Añade las dos herramientas al catálogo del LLM (con confirmación), los resúmenes de
confirmación, y `_delegate_read`, que envía el submit y sondea `outcome` hasta obtener
el contenido (con cancelación y timeout).

**Files:**
- Modify: `server/atlas_relay.py` (constantes de herramientas, `tool_defs`, `_confirm_summary`, `execute_tool`, nuevo `_delegate_read`)
- Modify: `server/test_hefesto_delegation.py` (tests nuevos)

**Interfaces:**
- Consumes: journal v2 con `hefesto.list_dir`/`hefesto.read_file` (Task 3); `new_id`, `hefesto.Peer`, `dlog`, `self.cancelled` (existentes).
- Produces:
  - `tool_defs()` devuelve también `hefesto_list_dir` y `hefesto_read_file` (solo con journal/dispositivo), ambas `requiresConfirmation: True`.
  - `Brain._delegate_read(self, tool_id: str, job: dict, args: dict) -> str`.

- [ ] **Step 1: Añadir las dos entradas del catálogo**

En `server/atlas_relay.py`, junto a `HEFESTO_TOOL` (línea ~168), añade:

```python
HEFESTO_LIST_TOOL = {
    "id": "hefesto_list_dir",
    "risk": "external",
    "requiresConfirmation": True,
    "description": ("Pide a Hefesto listar un directorio del Mac. Usa una ruta "
                    "absoluta dentro de las raíces autorizadas (Documents, Desktop, "
                    "Downloads, Pictures, Music, Movies). Cada listado pide "
                    "confirmación al usuario."),
    "schema": {
        "type": "object",
        "properties": {
            "path": {"type": "string",
                     "description": "ruta absoluta del directorio a listar"},
        },
        "required": ["path"],
    },
}

HEFESTO_READ_TOOL = {
    "id": "hefesto_read_file",
    "risk": "external",
    "requiresConfirmation": True,
    "description": ("Pide a Hefesto leer un archivo de texto del Mac. Usa una ruta "
                    "absoluta dentro de las raíces autorizadas (Documents, Desktop, "
                    "Downloads, Pictures, Music, Movies). El contenido saldrá del Mac "
                    "hacia el modelo, así que cada lectura pide confirmación. "
                    "Máximo 16 KiB; los binarios se rechazan."),
    "schema": {
        "type": "object",
        "properties": {
            "path": {"type": "string",
                     "description": "ruta absoluta del archivo a leer"},
        },
        "required": ["path"],
    },
}

HEFESTO_TOOLS = [HEFESTO_TOOL, HEFESTO_LIST_TOOL, HEFESTO_READ_TOOL]
```

Y en `tool_defs` (línea ~780):

```python
    def tool_defs(self) -> list[dict]:
        if self._hefesto_journal is not None and self._hefesto_device is not None:
            return TOOL_DEFS + HEFESTO_TOOLS
        return TOOL_DEFS
```

- [ ] **Step 2: Añadir los resúmenes de confirmación**

En `_confirm_summary` (línea ~1059), antes del `return (f"Ejecutar {tool_id}", "external")` final, añade:

```python
        if tool_id == "hefesto_list_dir":
            return (f"Listar «{args.get('path', '?')}»", "external")
        if tool_id == "hefesto_read_file":
            return (f"Leer «{args.get('path', '?')}» — el contenido saldrá del Mac hacia el modelo", "external")
```

- [ ] **Step 3: Rutear en `execute_tool`**

En `execute_tool` (línea ~800), tras el caso `hefesto_echo`, añade:

```python
        if tool_id == "hefesto_list_dir":
            return self._delegate_read("hefesto.list_dir", job, args)
        if tool_id == "hefesto_read_file":
            return self._delegate_read("hefesto.read_file", job, args)
```

- [ ] **Step 4: Implementar `_delegate_read`**

Añade el método tras `_delegate_echo` (línea ~866):

```python
    def _delegate_read(self, tool_id: str, job: dict, args: dict) -> str:
        journal, device_id = self._hefesto_journal, self._hefesto_device
        if journal is None or device_id is None:
            return "Hefesto no está disponible (desactivado)"
        path = str(args.get("path", ""))
        if not path.startswith("/") or "\x00" in path or len(path.encode("utf-8")) > 1024:
            return "ruta inválida (debe ser una ruta absoluta dentro de las raíces autorizadas)"
        submission = {
            "protocolVersion": 2,
            "submissionId": new_id("sub"),
            "requestId": job["request_id"],
            "sessionId": job["session_id"],
            "taskId": new_id("tsk"),
            "toolId": tool_id,
            "arguments": {"path": path},
        }
        status, response = journal.dispatch(
            hefesto.Peer("controller", device_id), "POST",
            "/v1/hefesto/submit", json.dumps(submission).encode("utf-8"))
        if status != 200:
            dlog(self, "HEFESTO",
                 f"read submit rejected error={response.get('error')} request={job['request_id']}")
            return f"Hefesto no está disponible ({response.get('error', 'desconocido')})"
        invocation_id = response["invocationId"]
        deadline = time.monotonic() + 15.0
        while time.monotonic() < deadline:
            with self.lock:
                if job["request_id"] in self.cancelled:
                    return "cancelada a mitad de turno"
            status, outcome = journal.dispatch(
                hefesto.Peer("controller", device_id), "POST",
                "/v1/hefesto/outcome",
                json.dumps({"protocolVersion": 2,
                            "invocationId": invocation_id}).encode("utf-8"))
            if status == 200 and outcome.get("state") in ("succeeded", "expired", "cancelled", "unknown"):
                if outcome["state"] == "succeeded":
                    return str(outcome.get("outputText") or "")
                if outcome["state"] == "expired":
                    return "Hefesto no respondió a tiempo y la lectura caducó."
                return "La lectura se canceló."
            time.sleep(0.5)
        return "Hefesto no respondió a tiempo (timeout de lectura)"
```

- [ ] **Step 5: Escribir los tests del relay**

En `server/test_hefesto_delegation.py`, añade una clase de tests. Reutiliza `make_brain`, `JOB` y el `FakeJournal` existente (que ya soporta `submit` con cola de respuestas y `outcome` por `invocationId`), más un fake que devuelve el estado en varias fases:

```python
class ReadFlowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-read-")
        self.addCleanup(self.temp.cleanup)
        self.brain = make_brain(self.temp.name)
        self.brain._hefesto_journal = FakeJournal()
        self.brain._hefesto_device = "mac-one"

    def test_read_file_envia_submit_con_path_y_devuelve_el_contenido(self):
        journal = self.brain._hefesto_journal
        journal.accept("inv-9")
        journal.outcomes["inv-9"] = {"state": "succeeded", "outputText": "contenido"}
        result = self.brain.execute_tool("hefesto_read_file",
                                         {"path": "/Users/x/Documents/a.txt"}, JOB)
        self.assertEqual(result, "contenido")
        self.assertEqual(journal.submits[0]["toolId"], "hefesto.read_file")
        self.assertEqual(journal.submits[0]["arguments"], {"path": "/Users/x/Documents/a.txt"})

    def test_read_con_ruta_relativa_no_envia_submit(self):
        result = self.brain.execute_tool("hefesto_read_file", {"path": "relativo"}, JOB)
        self.assertTrue(result.startswith("ruta inválida"))
        self.assertEqual(self.brain._hefesto_journal.submits, [])

    def test_read_con_submit_rechazado_devuelve_no_disponible(self):
        journal = self.brain._hefesto_journal
        journal.reject("device_suspended")
        result = self.brain.execute_tool("hefesto_read_file",
                                         {"path": "/Users/x/Documents/a.txt"}, JOB)
        self.assertTrue(result.startswith("Hefesto no está disponible"))

    def test_read_sin_journal_devuelve_desactivado(self):
        self.brain._hefesto_journal = None
        self.brain._hefesto_device = None
        result = self.brain.execute_tool("hefesto_read_file", {"path": "/a"}, JOB)
        self.assertEqual(result, "Hefesto no está disponible (desactivado)")

    def test_catalogo_incluye_las_dos_lecturas_con_confirmacion(self):
        ids = {t["id"]: t for t in self.brain.tool_defs()}
        self.assertIn("hefesto_list_dir", ids)
        self.assertIn("hefesto_read_file", ids)
        self.assertTrue(ids["hefesto_read_file"]["requiresConfirmation"])

    def test_confirm_summary_de_lectura_menciona_la_exportacion(self):
        summary, _ = self.brain._confirm_summary("hefesto_read_file", {"path": "/x/y.txt"})
        self.assertIn("Leer «/x/y.txt»", summary)
```

- [ ] **Step 6: Ejecutar las pruebas y verificar**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: `OK` con las pruebas nuevas (101 anteriores + 6 nuevas).

- [ ] **Step 7: Commit**

```bash
git add server/atlas_relay.py server/test_hefesto_delegation.py
git commit -m "feat(hefesto): lectura síncrona con confirmación desde el relay"
```

---

### Task 6: Contratos y documentación

Actualiza la documentación del protocolo y los contratos para reflejar v2 y las dos
herramientas nuevas.

**Files:**
- Modify: `docs/hefesto-plan.md` (§5)
- Modify: `docs/contracts.md` (sección Hefesto)

**Interfaces:**
- Consumes: nada ejecutable.
- Produces: documentación alineada con el código de las Tasks 1–5.

- [ ] **Step 1: Actualizar `docs/hefesto-plan.md` §5**

En la sección «5. Protocolo de base v1 (solo eco)»:
- Cambia el título a «5. Protocolo v2 (eco y lectura)».
- En la tabla de rutas, amplía la fila de `submit` para indicar `toolId: hefesto.echo | hefesto.list_dir | hefesto.read_file` con `arguments: {text}` (eco) o `{path}` (lectura/listado), y `output: {text}` (≤64 KiB).
- Añade un párrafo corto: versión 2; `path` absoluto (≤1024 bytes, sin NUL); lectura ≤16 KiB; listado ≤100 entradas; binarios rechazados como «no legible»; los errores de lectura vuelven como `succeeded` con el mensaje en `output.text`.

- [ ] **Step 2: Actualizar `docs/contracts.md`**

En la sección «Hefesto v1 — base de eco» (línea ~176), retitúlala a «Hefesto v2 — eco y lectura» y añade el catálogo de herramientas (tres `toolId`, shapes de `arguments`, tope de `output.text` 64 KiB) apuntando a `docs/hefesto-plan.md` §5.

- [ ] **Step 3: Commit**

```bash
git add docs/hefesto-plan.md docs/contracts.md
git commit -m "docs(hefesto): protocolo v2 con herramientas de lectura"
```

---

### Task 7: Verificación global, despliegue y registro

Ejecuta la verificación completa, despliega el relay y el bundle, prueba en vivo una
lectura con el usuario, y registra la entrega.

**Files:**
- Modify: `docs/implemented.md`, `docs/implementation-plan.md`, `docs/hefesto-plan.md` §8

**Interfaces:**
- Consumes: Tasks 1–6.
- Produces: entrega desplegada y registrada.

- [ ] **Step 1: Verificación global**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: `OK` (todas).

Run: `cargo test -p atlas-mac` y `cargo test --workspace` y `cargo check --workspace`
Expected: verdes.

Run: `cd apps/panel && npm run build`
Expected: build correcto (el panel no cambia, pero se comprueba regresión).

- [ ] **Step 2: Desplegar el relay**

Run: `./scripts/deploy-relay.sh`
Expected: push + pull + `atlas-relay` activo + `/v1/health` OK.

- [ ] **Step 3: Reconstruir el bundle de la app**

Reconstruye con Apple Development y verifica firma profunda (identidad en
`security find-identity -v -p codesigning`), instala en `/Applications/Atlas.app`
y pide al usuario que lo abra.

- [ ] **Step 4: Prueba en vivo de lectura (con el usuario)**

Con la app abierta y Hefesto activo, pide al usuario un encargo que lea un archivo de
texto pequeño en `Documents` (p.ej. «lee Documents/notas.txt y dime qué dice»). Verifica:
aparece la confirmación (app o Telegram) con la ruta; al aceptar, el contenido llega al
modelo y la respuesta del LLM lo usa; al denegar, no se lee nada. Prueba también una ruta
fuera de raíces (debe responder «fuera de las raíces autorizadas»). Al terminar, auditoría
de procesos (`ps`/`pgrep`/`launchctl list`) y reporte de qué queda vivo.

- [ ] **Step 5: Registrar la entrega**

En `docs/hefesto-plan.md` §8, añade la entrada 2026-10-30 con los commits exactos, las
pruebas y la prueba en vivo. En `docs/implemented.md` añade «Lectura segura de archivos
de Hefesto» con los nombres de commit. En `docs/implementation-plan.md` marca la puerta 4
(corte de lectura) como cerrada.

- [ ] **Step 6: Commit y push (con permiso del usuario)**

```bash
git add docs/implemented.md docs/implementation-plan.md docs/hefesto-plan.md
git commit -m "docs: registrar la lectura segura de archivos de Hefesto"
git push origin main
```

(Commit y push solo con permiso explícito del usuario, según AGENTS.md.)
