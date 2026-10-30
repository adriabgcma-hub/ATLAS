# Especificación: lectura segura de archivos de Hefesto (puerta 4, corte 1)

- Fecha: 2026-10-30
- Estado: diseño aprobado por el usuario; pendiente de revisión y plan de implementación
- Fuente: `docs/hefesto-plan.md` §4 (matriz de consentimiento) y §6 (entrega 4), y decisiones de la sesión de diseño

## 1. Contexto y objetivo

Hefesto hoy solo ejecuta `hefesto.echo` (texto de prueba). La puerta siguiente del
plan es la **4: Lecturas y archivos**, que empieza por la lectura. Este diseño
cubre el **corte 1: solo lectura segura**: listar directorios y leer archivos
dentro de raíces autorizadas, con consentimiento de exportación por operación y
resultado síncrono al modelo. Sin escritura, sin terminal, sin accesibilidad.

Objetivo: que el LLM pueda, con permiso del usuario, **listar** un directorio
autorizado y **leer** un archivo de texto para razonar sobre su contenido
(resumir, responder, comparar), sin que el contenido salga del Mac sin
consentimiento explícito por lectura.

## 2. Requisitos

1. Dos herramientas nuevas junto al eco existente:
   - `hefesto.list_dir` — `{path}` → lista de entradas (nombre, tipo, tamaño).
   - `hefesto.read_file` — `{path}` → contenido del archivo como texto.
2. **Raíces autorizadas**: Documents, Desktop, Downloads, Pictures, Music y Movies
   del usuario (macOS). «Proyectos dados de alta» queda para un corte posterior.
3. **Contención real, no prefijos**: el dispositivo resuelve la ruta canónica
   (resolviendo enlaces) y verifica que el objeto abierto queda dentro de una raíz
   autorizada. Un enlace que apunta fuera se rechaza.
4. **Consentimiento por lectura**: cada `list_dir`/`read_file` pide aprobación al
   usuario (app o Telegram) antes de que el contenido salga del Mac; denegar o
   caducar cancela sin leer.
5. **Resultado síncrono**: el relay espera el resultado (con timeout) y lo devuelve
   al LLM como resultado de la herramienta en el mismo turno.
6. **Límites**: lectura acotada (16 KiB por defecto); listado acotado (100 entradas);
   los archivos binarios se rechazan con «no legible» en vez de volcar basura.
7. El eco sigue funcionando igual; la delegación asíncrona (`_delegations`/tracker)
   no cambia para el eco.
8. Sin regresiones: chat, calendario, Telegram, identidad estable y las 93 pruebas
   Python + pruebas Rust/TS actuales siguen pasando.

## 3. Diseño

### 3.1 Herramientas y esquema de argumentos

| toolId (journal) | Catálogo Groq (relay) | `arguments` | `output.text` |
| --- | --- | --- | --- |
| `hefesto.echo` | `hefesto_echo` | `{text}` (≤256 B) | eco |
| `hefesto.list_dir` | `hefesto_list_dir` | `{path}` | listado como texto |
| `hefesto.read_file` | `hefesto_read_file` | `{path}` | contenido como texto |

- `path`: cadena no vacía, ≤1024 bytes UTF-8, sin NUL, **absoluta** (empieza por
  `/`). El dispositivo hace la validación de contención (autoritativa); el relay
  solo valida forma (no resuelve rutas: corre en `atlas-hub`, no en el Mac).
- El output de todas las herramientas sigue el shape `{text}` (texto acotado).

### 3.2 Protocolo v2

- `protocolVersion` pasa a **2**. El eco mantiene sus shapes (`{text}` en
  arguments y output) sin cambio semántico; solo cambia el número de versión.
- `toolId` admite `hefesto.echo`, `hefesto.list_dir`, `hefesto.read_file`.
- `arguments` pasa de «solo `{text}`» a **payload por herramienta**: `{text}` para
  eco, `{path}` para list/read. En el wire de Rust se modela como enum `ToolArguments`
  (untagged: `Echo {text}` y `Path {path}`), con `deny_unknown_fields` para rechazar
  mezclas (`{text, path}` queda fuera de ambas variantes).
- `output.text`: tope de wire 64 KiB (presupuesto del plan §3); el ejecutor aplica
  topes por herramienta más estrictos (eco ≤256 B por su argumento, lectura ≤16 KiB).
- Espejo obligatorio (regla de contratos): `crates/atlas-mac/src/wire.rs`,
  `server/hefesto.py`, `docs/hefesto-plan.md` §5 y `docs/contracts.md` (§ Hefesto).

### 3.3 Raíces seguras (dispositivo, Rust)

- Raíces = los 6 directorios estándar del usuario (vía la API de directorios del
  sistema, no rutas hardcodeadas).
- Resolución: `canonicalize` de la ruta pedida (resuelve enlaces y `..`), y
  verificación de que el resultado está dentro de una raíz canónica (comparación de
  componentes, no prefijo de texto). Solo se leen **archivos regulares** y se
  listan **directorios**; cualquier otro tipo (sockets, dispositivos, FIFO) se
  rechaza.
- Sin acceso a fs para el webview: la lectura ocurre en el ejecutor Rust nativo.

### 3.4 Despachador en el dispositivo (Rust)

- `EchoExecutor` se generaliza a un despachador de herramientas: `execute` decide
  por `toolId` y devuelve `{text}`.
  - `list_dir`: `read_dir` + metadatos; hasta 100 entradas, nombres y tamaños;
    orden determinista; errores de permisos como texto de error estable.
  - `read_file`: `read_to_string` acotado a 16 KiB; si el archivo es mayor, error
    con el tamaño real («demasiado grande: N bytes, máximo 16 KiB»); si contiene
    NUL o no es UTF-8 válido, error «binario/no legible».
- El resultado se devuelve por el `result` existente (`output.text`); la idempotencia,
  el TTL de 15 s y la deduplicación del protocolo v1 se conservan.
- **Errores de lectura** (fuera de raíz, demasiado grande, binario, sin permisos,
  ruta inexistente): se devuelven como `succeeded` con el mensaje de error en
  `output.text`. El protocolo no tiene estado `failed` para resultados; el LLM lee
  el mensaje y lo narra al usuario.

### 3.5 Relay: catálogo, confirmación y espera síncrona

1. **Catálogo**: `tool_defs()` añade `hefesto_list_dir` y `hefesto_read_file`
   (solo cuando hay journal/dispositivo, como `hefesto_echo`), con
   `requiresConfirmation: True` y descripciones que digan al modelo que use rutas
   absolutas dentro de las raíces autorizadas y que cada lectura pide permiso.
2. **Confirmación**: `_confirm_summary` gana casos para list/read —
   «Listar `<path>`» / «Leer `<path>` — el contenido saldrá del Mac hacia el
   modelo». Reutiliza `ask_confirmation` (ConfirmCard en la app, botones en
   Telegram). Denegar/caducar devuelve «denegada por el usuario» sin leer.
3. **Espera síncrona** (`_delegate_read`): valida forma de `path` → `submit` con
   `toolId`/`arguments {path}` → sondea `outcome` cada 500 ms hasta 15 s (mismo
   TTL de invocación), abortando si el turno se cancela (`request_id` en
   `self.cancelled`). Terminal `succeeded` → devuelve `outputText` como resultado
   de la herramienta; `expired`/`cancelled`/`unknown` o timeout → mensaje de
   estado. **No** se registra en `_delegations` ni genera mensaje «Hefesto:» aparte
   (el eco asíncrono sigue intacto).
4. El contenido va al modelo solo en el turno que lo lee; lo que persiste en el
   historial de sesión es la **respuesta del LLM**, no el archivo.

### 3.6 Límites y rechazo de binarios

- `READ_MAX_BYTES = 16 KiB`, `LIST_MAX_ENTRIES = 100`,
  `READ_WAIT_TIMEOUT_S = 15`, `OUTCOME_POLL_EVERY = 0.5 s`,
  `PATH_MAX_BYTES = 1024`, `MAX_OUTPUT_BYTES = 64 KiB`.
- Archivos con NUL en los primeros bytes o UTF-8 inválido → «binario/no legible».

## 4. Contratos y documentación

- `docs/hefesto-plan.md` §5: tabla de rutas con los nuevos `toolId`, y versión 2.
- `docs/contracts.md`: sección Hefesto (catálogo de herramientas y versión 2).
- `docs/implemented.md`, `docs/implementation-plan.md` y `docs/hefesto-plan.md` §8
  al cerrar la entrega (commits exactos).
- `docs/relay.md`: no cambia (los endpoints HTTP del relay no añaden rutas nuevas;
  el catálogo de Groq se describe en `docs/hefesto-plan.md`).

## 5. Fuera de alcance

- Escritura (crear, sobrescribir, mover, renombrar, borrar) — puerta 4, cortes posteriores.
- «Proyectos dados de alta» (registro de raíces extra) — corte posterior.
- Terminal, apps/multimedia y accesibilidad (puertas 5–7).
- Consentimiento persistente por raíz (esta entrega es por lectura).
- Reactivación remota de Hefesto (sigue manual y en la app).

## 6. Decisiones y trade-offs

1. **Versión 2 en vez de v1 aditivo**: el shape de `arguments` cambia (de `{text}`
   a por-herramienta); versionar es honesto y el coste es mecánico (una constante
   + fixtures). El eco conserva su shape.
2. **Espera síncrona reutilizando `outcome`**: no hay operación nueva en el
   journal; el relay sondea la lectura del resultado ya existente. Coste: latencia
   ~1–2 s por lectura (el dispositivo sondea `next` cada 1 s), acotada por el TTL
   de 15 s.
3. **Consentimiento por lectura** (no persistente): conservador y alineado con la
   matriz del plan («individual»); si resulta pesado, se propone persistente por
   raíz en un corte posterior.
