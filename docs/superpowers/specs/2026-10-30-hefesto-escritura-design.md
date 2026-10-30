# Especificación: escritura segura de archivos de Hefesto (puerta 4, corte 2)

- Fecha: 2026-10-30
- Estado: diseño aprobado por el usuario; pendiente de revisión del spec y plan de implementación
- Fuente: `docs/hefesto-plan.md` §3 (límites), §4 (matriz de consentimiento) y §6 (puerta 4), y las decisiones de la sesión de diseño

## 1. Contexto y objetivo

La lectura segura (corte 1) permite listar y leer dentro de raíces autorizadas con
consentimiento por operación y resultado síncrono. Este corte añade la **escritura
segura**: crear archivos nuevos sin sobrescribir, sobrescribir con un plan exacto
verificado y mover/renombrar dentro de las mismas raíces. Sin borrado, sin permisos,
sin creación de directorios ni de enlaces.

Objetivo: que el LLM pueda, con aprobación explícita del usuario y sin riesgo
destructivo silencioso, guardar texto en el Mac (notas, configuraciones, documentos
pequeños) y reorganizar archivos existentes dentro de lo autorizado.

## 2. Requisitos

1. Dos herramientas nuevas en el catálogo del LLM:
   - `hefesto.write_file` — `{path, content, expectedHash}`: sin hash crea (falla si
     existe); con hash sobrescribe solo si el archivo sigue teniendo ese sha256.
   - `hefesto.rename` — `{from, to}`: mover o renombrar; el destino no puede existir.
2. Una operación interna del dispositivo, **no visible al LLM**:
   `hefesto.file_info` — `{path}` → existencia, tamaño y sha256, sin contenido. La usan
   el relay (para el plan) y la validación de las herramientas.
3. **Consentimiento en las tres operaciones** (decisión del usuario): crear,
   sobrescribir y mover/renombrar piden confirmación con resumen exacto. Riesgo `write`.
4. **Plan exacto con control optimista**: la tarjeta de sobrescritura muestra tamaño
   actual, hash actual (12 hex) y tamaño nuevo; el dispositivo rechaza la escritura si
   el hash actual no coincide con el confirmado.
5. **Escritura atómica y exclusiva**: crear publica sin poder truncar un destino
   existente; sobrescribir escribe un temporal en el mismo directorio y lo publica con
   `rename` atómico; mover usa `rename` atómico y solo en el mismo volumen.
6. **Contención real** de origen y destino (canónica, por componentes, sin enlaces) en
   las raíces autorizadas del usuario.
7. **El journal no persiste el contenido** de las escrituras: guarda toolId, rutas,
   tamaño y hashes; el texto vive solo en memoria para entregarlo al dispositivo. Tras
   un reinicio con una escritura sin entregar, la invocación queda `unknown` y no se
   reenvía.
8. Límites: contenido ≤16 KiB de texto UTF-8 sin NUL; rutas ≤1024 B; hash de archivos
   existentes para el plan ≤64 MiB; cuerpo del protocolo ≤128 KiB.
9. Sin regresiones: eco, lectura, chat, calendario, Telegram, identidad y las pruebas
   actuales (Rust, Python, frontend) siguen pasando.

## 3. Diseño

### 3.1 Herramientas y esquema de argumentos

| toolId | Catálogo Groq | `arguments` | `output` |
| --- | --- | --- | --- |
| `hefesto.write_file` | `hefesto_write_file` | `{path, content, expectedHash\|null}` | `{text}` resumen |
| `hefesto.rename` | `hefesto_rename` | `{from, to}` | `{text}` resumen |
| `hefesto.file_info` | — (interna) | `{path}` | `{exists, size\|null, sha256\|null}` |

- `path`/`from`/`to`: cadena no vacía, ≤1024 bytes UTF-8, sin NUL, **absoluta**. La
  contención la decide el dispositivo (autoritativo); el relay solo valida forma.
- `content`: cadena UTF-8 ≤16 KiB (16 384 bytes), sin NUL. Solo texto.
- `expectedHash`: `null` o 64 hex minúscula (sha256). `null` = crear (exclusivo).
- Resúmenes de salida (texto, sin contenido):
  - escritura: `Escrito «<path>» (<N> B, sha256 <12 hex>)`.
  - renombrado: `Movido «<from>» → «<to>» (<N> B)`.

### 3.2 Protocolo v3

- `protocolVersion` pasa a **3** en todas las solicitudes POST y respuestas. Eco y
  lectura mantienen shapes; solo cambia el número.
- `toolId` admitidos en `submit`: `hefesto.echo`, `hefesto.list_dir`,
  `hefesto.read_file`, `hefesto.write_file`, `hefesto.rename`, `hefesto.file_info`.
- `output` pasa a una unión validada por herramienta:
  - `{text}` (≤64 KiB) para eco, list_dir, read_file, write_file y rename.
  - `{exists, size, sha256}` para file_info (`size`/`sha256` `null` si no existe).
- `file_info` es una invocación normal del journal (submit → next → result → outcome),
  pero el relay no la expone al modelo. No requiere confirmación.
- El journal valida argumentos por herramienta y rechaza campos desconocidos.

### 3.3 Journal: contenido solo en memoria

- La fila de sqlite guarda el `submission` **sin** `content`, con `contentBytes` y
  `contentSha256`. El texto completo vive en un diccionario en memoria por
  `invocationId`.
- `next` entrega los argumentos completos desde memoria. Si la entrada no está (p.
  ej. tras reiniciar el proceso), la invocación se marca `unknown` y nunca se
  reenvía: mismo criterio que un resultado perdido.
- `submit` idempotente: mismo `submissionId` y mismos metadatos devuelve la misma
  invocación; si el contenido en memoria ya no existe, responde `unknown`.
- `outcome` sigue siendo lectura pura y devuelve el `output` tipado.

### 3.4 Relay: plan y confirmación

1. El modelo llama `hefesto_write_file` o `hefesto_rename`. El relay valida forma.
2. Plan (`file_info` interno, espera ≤5 s con sondeo 0,5 s):
   - `write_file` con `expectedHash: null`:
     - no existe → tarjeta «Crear «path» (N B)».
     - existe → se devuelve al modelo sin tarjeta: «ya existe; usa expectedHash para
       sobrescribir».
   - `write_file` con hash:
     - existe → tarjeta «Sobrescribir «path» (actual X B, sha256 abc123def456… →
       nuevo N B)».
     - no existe → al modelo: «el archivo ya no existe».
   - `rename`: origen existe y destino no → tarjeta «Mover «from» → «to» (N B,
     sha256 …)». Origen inexistente o destino existente → al modelo sin tarjeta.
   - `file_info` fallido (offline, ruta fuera de raíces, >64 MiB) → al modelo con el
     motivo, sin tarjeta.
3. `ask_confirmation` (app o Telegram, riesgo `write`) → denegar/caducar devuelve
   «denegada por el usuario» sin escribir.
4. Al aprobar: `_delegate` síncrono (mismo que lectura, 15 s) con los argumentos
   exactos; el resultado vuelve al modelo en el mismo turno. No se registra en
   `_delegations` ni genera mensaje «Hefesto:».
5. Cancelación a mitad de turno: se revalida antes de cada submit y durante la espera,
   como en lectura.

### 3.5 Dispositivo (Rust)

- `tools.rs` gana `file_info`, `write_file` y `rename`, con `Roots` compartido.
- **Resolución de destino**: se canonicaliza el directorio padre y se comprueba
  contención por componentes; la ruta final no puede ser ni atravesar un enlace
  (`symlink_metadata`); el padre debe existir y ser directorio.
- **Crear (exclusivo)**: temporal `.atlas-write-<pid>-<nanos>.tmp` creado con
  `create_new` en el mismo directorio, escritura completa y `fsync`; publicación con
  `hard_link(temp, destino)` (falla si existe) y borrado del temporal; si el sistema
  de archivos no admite enlaces duros, la escritura falla sin publicar. Nunca abre el
  destino existente para escritura.
- **Sobrescribir**: solo archivo regular no enlace; se recalcula sha256 del actual y
  se compara con `expectedHash` (si no coincide → «el archivo cambió desde la
  confirmación», sin tocar nada); temporal + `fsync` + `rename` atómico sobre el
  destino.
- **Mover/renombrar**: origen regular no enlace, destino inexistente, ambos padres en
  raíces; `std::fs::rename`; si cruza volúmenes (`EXDEV`) → «no se puede mover entre
  volúmenes».
- **file_info**: archivo regular no enlace; tamaño; sha256 en streaming con tope de
  64 MiB («demasiado grande para planificar»).
- Errores se devuelven como texto al modelo (sin rutas del sistema ni secretos),
  reutilizando el patrón de lectura: «fuera de las raíces autorizadas», «ya existe»,
  «no es un archivo regular», etc.
- `Executor` enruta los tres toolIds nuevos; el eco y la lectura no cambian.

### 3.6 Auditoría y privacidad

- Journal: toolId, rutas, `expectedHash`, `contentBytes`, `contentSha256`, estados y
  códigos. Nunca el contenido.
- Resultado al modelo: ruta solicitada, tamaño y hash corto. Sin contenido ajeno.
- Los resúmenes de tarjeta muestran ruta, tamaños y hash corto; nunca el contenido
  completo.

## 4. Fuera de alcance

- Borrado, `chmod`/propietarios, creación de directorios, enlaces simbólicos.
- Escritura binaria o base64; archivos >16 KiB por operación.
- Mover entre volúmenes (se rechaza con error estable).
- Reanudación de una escritura tras reinicio del relay (queda `unknown`).
- UI nueva: se reutiliza la tarjeta de confirmación existente.

## 5. Pruebas

- **Rust** (`tools.rs`): crear y verificar contenido; crear sobre existente falla sin
  modificarlo; sobrescribir con hash correcto; hash obsoleto rechaza y no modifica;
  `file_info` (existe/tamaño/hash, enlace rechazado, >tope); contención fuera de
  raíces y por enlace; rename correcto, destino existente, fuera de raíces.
- **Python** (`test_hefesto.py`): validación v3 de los tres toolIds; el contenido no
  aparece en sqlite; `next` sirve el contenido desde memoria; reinicio con escritura
  pendiente → `unknown` y sin reenvío; idempotencia por `submissionId`; validación del
  `output` tipado.
- **Relay** (`test_hefesto_delegation.py`): catálogo (write/rename sí, file_info no);
  resúmenes exactos; plan de creación, sobrescritura y movimiento; «ya existe»/«destino
  existente» sin tarjeta; hash obsoleto → rechazo en texto; allow/deny/caducidad;
  cancelación.
- **En vivo**: crear una nota, sobrescribirla con el hash de la tarjeta, intentar con
  un hash obsoleto (rechazo), mover/renombrar, y ruta fuera de raíces (rechazo). Se
  verifica en la app y por el flujo de eventos del relay.

## 6. Criterios de aceptación

- Crear nunca trunca ni sobrescribe un destino existente, ni siquiera con el modelo
  insistiendo.
- Una sobrescritura confirmada con un hash que ya no coincide no modifica el archivo.
- Mover no pisa un destino existente y no sale de las raíces.
- Ningún contenido de escritura queda en sqlite ni en logs.
- Las tres operaciones piden confirmación con el resumen exacto definido.
- Eco y lectura siguen funcionando; pruebas actuales verdes.
