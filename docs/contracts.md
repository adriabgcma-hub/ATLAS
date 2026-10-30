# Contratos del sistema

Estos contratos describen las fronteras públicas de Atlas. Son una especificación inicial; cuando se implementen deben convertirse en tipos estrictos compartidos o equivalentes validados en Rust y TypeScript.

## Identificadores

Todos los eventos de una operación deben incluir:

```ts
type Correlation = {
  requestId: string
  sessionId: string
  timestamp: string // ISO-8601 UTC
}
```

Los identificadores se generan en el núcleo y nunca contienen secretos ni datos personales.

## Ciclo de una sesión

```ts
type SessionState =
  | 'idle'
  | 'thinking'
  | 'awaiting_confirmation'
  | 'executing'
  | 'responding'
  | 'cancelled'
  | 'completed'
  | 'failed'
```

```ts
type SessionEvent = Correlation & {
  type: 'session.state_changed'
  state: SessionState
  message?: string
}
```

El frontend solo representa este estado; no puede forzar una transición de ejecución sin pasar por IPC y permisos.

En `executing`, `message` lleva el `toolId` del catálogo (p. ej.
`hefesto_read_file`), nunca rutas ni contenido: la app lo usa para la órbita de
fase (Hefesto en violeta, otras herramientas en azul). En `failed`, `message`
es el código estable del error. Un estado terminal limpia el mensaje.

## Canales de eventos IPC

El nombre del canal Tauri no puede contener puntos (Tauri 2 solo admite
caracteres alfanuméricos y `-`, `/`, `:`, `_`); si se viola, `listen` y `emit`
fallan y el frontal se queda sin eventos en silencio. Los canales son por tanto:

| Contrato (campo `type` del payload) | Canal Tauri |
| --- | --- |
| `session.state_changed` | `session-state-changed` |
| `conversation.message_appended` | `conversation-message-appended` |
| `permission.confirmation_required` | `permission-confirmation-required` |
| `hefesto.state_changed` | `hefesto-state-changed` |

El contrato sigue siendo el nombre con puntos: viaja dentro del payload como
`type`. El test `canales_ipc_aceptados_por_tauri` (`src-tauri/src/lib.rs`)
vigila los nombres de canal.

## Mensajes IPC

```ts
type UserMessage = Correlation & {
  type: 'conversation.send'
  content: string
  attachments?: Array<{ id: string; mediaType: string; name: string }>
}

type CancelRequest = Correlation & {
  type: 'conversation.cancel'
}
```

El núcleo debe rechazar mensajes sin `requestId`, con contenido vacío o con adjuntos que no hayan sido autorizados por la sesión.

## Herramientas

```ts
type RiskLevel = 'read' | 'write' | 'external' | 'destructive'

type ToolManifest = {
  id: string
  version: string
  description: string
  risk: RiskLevel
  inputSchema: unknown
  outputSchema: unknown
  requiresConfirmation: boolean
}

type ToolCall = Correlation & {
  type: 'tool.call'
  toolId: string
  arguments: unknown
}

type ToolResult = Correlation & {
  type: 'tool.result'
  toolId: string
  status: 'succeeded' | 'failed' | 'rejected' | 'cancelled'
  output?: unknown
  errorCode?: string
}
```

Los argumentos se validan contra `inputSchema` antes de ejecutar el adaptador. Los resultados externos se normalizan antes de devolverse al modelo.

## Confirmaciones

```ts
type ConfirmationRequest = Correlation & {
  type: 'permission.confirmation_required'
  toolId: string
  summary: string
  risk: RiskLevel
  expiresAt: string
}

type ConfirmationResponse = Correlation & {
  type: 'permission.respond'
  decision: 'allow_once' | 'deny'
}
```

Nunca se debe interpretar la ausencia de respuesta como autorización. Las confirmaciones caducan y se vinculan a un único `requestId`.

## Origen del mensaje

```ts
type Source = 'local' | 'telegram'
```

`local` es la barra de input de la app; `telegram` es el bot consultado por
polling saliente. El núcleo trata ambos igual; solo cambia el origen registrado.

## Mensajes añadidos a la conversación

```ts
type MessageRole = 'user' | 'assistant'

type Agent = 'atlas' | 'hefesto'

type MessageAppended = Correlation & {
  type: 'conversation.message_appended'
  role: MessageRole
  content: string // no vacío, máximo 4000 caracteres
  source: Source
  /** Agente emisor del mensaje; ausente = Atlas. */
  agent?: Agent
  /** Correlación con la delegación (taskId del protocolo Hefesto). */
  taskId?: string
}
```

El frontend suscribe este evento y representa el mensaje sin distinguir el origen:
la cadencia visual es idéntica en ambos canales. Un continuador de Hefesto llega
como `message_appended` con `agent: 'hefesto'` y **requestId propio** (nunca
reutiliza el del turno): el interino y el resultado no colisionan. Los
continuadores no emiten eventos de estado de sesión.

## Errores

Los errores expuestos al frontend deben ser seguros para mostrar:

```ts
type AtlasError = {
  code: string
  message: string
  retryable: boolean
  requestId?: string
}
```

Los detalles internos, rutas privadas, tokens y respuestas completas de proveedores solo pueden aparecer en logs locales protegidos, y nunca en eventos de UI.

## Hefesto v3 — eco, lectura y escritura

Contrato independiente del chat, especificado en [Hefesto §5](./hefesto-plan.md#5-protocolo-v3-eco-lectura-y-escritura).
Usa `protocolVersion: 3`, IDs opacos, campos estrictos, cuerpo máximo 128 KiB y
`output` tipado por herramienta. Seis herramientas sobre el mismo protocolo:
`hefesto.echo` (`{text}`, ≤256 B), `hefesto.list_dir` y `hefesto.read_file`
(`{path}` absoluto, raíces autorizadas Documents/Desktop/Downloads/Pictures/
Music/Movies), `hefesto.write_file` (`{path, content, expectedHash|null}`),
`hefesto.rename` (`{from, to}`) y la interna `hefesto.file_info` (`{path}`). El relay
expone al modelo `hefesto_echo` (asíncrono), `hefesto_list_dir` y
`hefesto_read_file` (lectura síncrona, con confirmación por operación),
`hefesto_write_file` (`{path, content, mode}`) y `hefesto_rename`;
`hefesto.file_info` no está en el catálogo.

Operación del journal usada por el controlador: `POST /v1/hefesto/outcome` (rol controller)
`{invocationId}` → `{protocolVersion, invocationId, state, output|null}`.
Lectura pura del journal. `output` es una unión validada por herramienta:

```ts
type HefestoOutput =
  | { text: string } // eco, list_dir, read_file, write_file y rename (≤64 KiB)
  | { exists: boolean; size: number | null; sha256: string | null } // file_info
```

### Escritura

`hefesto_write_file` recibe `{path, content, mode}` (`create` falla si el destino
existe; `overwrite` lo reemplaza) y el relay inyecta en el journal el
`expectedHash` planificado —el sha256 actual o `null` para crear—; el modelo nunca
maneja hashes. `hefesto_rename` recibe `{from, to}` y no pisa un destino existente.
`hefesto.file_info` es una invocación normal pero no visible al modelo: responde
`{exists, size, sha256}` o `{text}` cuando rechaza la ruta o el archivo. Crear,
sobrescribir y mover piden confirmación por operación con la tarjeta del plan; el
riesgo es `write`.

Límites: `content` ≤16 384 bytes UTF-8 sin NUL (solo texto); rutas absolutas
≤1024 bytes UTF-8 sin NUL; cuerpo del protocolo ≤128 KiB; `output.text` ≤64 KiB;
hash de archivos existentes para el plan ≤64 MiB.

Garantías del dispositivo: crear escribe un temporal en el mismo directorio, hace
`fsync` y publica con `hard_link` (nunca trunca ni pisa un destino); sobrescribir
recalcula el sha256 actual, lo compara con `expectedHash`, escribe temporal +
`fsync` y publica con `rename` atómico conservando el modo; mover usa `hard_link`
y elimina el origen (sin pisar) y solo dentro del mismo volumen.

Errores estables, devueltos como texto: «fuera de las raíces autorizadas», «ya
existe», «el archivo cambió desde la confirmación», «el destino ya existe», «no se
puede mover entre volúmenes» y «demasiado grande para planificar».

Resúmenes de resultado: `Escrito «<path>» (<N> B, sha256 <12 hex>)` y
`Movido «<from>» → «<to>» (<N> B)`. Tarjetas: `Crear «path» (N B)`,
`Sobrescribir «path» (actual X B, sha256 <12 hex>… → nuevo N B)` y
`Mover «from» → «to» (N B, sha256 <12 hex>…)`.

El contenido nunca se persiste en sqlite: la fila guarda el envío redactado con
`contentBytes` y `contentSha256`, y el texto vive solo en memoria del relay para
entregarlo al dispositivo. Si el proceso se reinicia antes de la entrega, la
invocación queda `unknown` y no se reenvía.

El dispositivo dentro de la app sí expone estado y control por IPC, siempre sobre
el bucle nativo (el webview nunca ve certificados, claves ni la URL interna):

```ts
type HefestoPhase =
  | 'unconfigured' | 'connecting' | 'suspended' | 'ready' | 'offline' | 'error'

type HefestoSnapshot = {
  phase: HefestoPhase
  deviceId: string | null
  generation: number | null
  lastEcho: string | null
  /** Código estable corto (p. ej. `transport_failed`); nunca rutas ni secretos. */
  detail: string | null
}

type HefestoEvent = HefestoSnapshot & { type: 'hefesto.state_changed' }
```

| Comando IPC | Entrada | Salida |
| --- | --- | --- |
| `hefesto_status` | — | `HefestoSnapshot` (estado actual, sin red) |
| `hefesto_resume` | — | — (reactivación manual tras una suspensión; el evento publica el cambio) |
| `hefesto_abort` | — | — (suspende en local al instante; el aviso al relay se reintenta) |

Al abrirse, la app activa el dispositivo sola si la suspensión anterior se había
levantado a mano; así Hefesto queda disponible por intención sin un paso extra.
Suspender —desde la app, Hub Panel o control remoto— deja un marcador
`hefesto-suspended` en el directorio de datos de la app
(`~/Library/Application Support/com.atlas.next/`). Mientras exista, arrancar no
reactiva nada: el chip muestra `Activar` y hasta que no se pulsa no se aceptan
encargos. Activar borra el marcador. El marcador no guarda datos, solo la
intención; si no se puede leer, se asume suspendido.

Las credenciales (`ATLAS_HEFESTO_CA`, `ATLAS_HEFESTO_CERT`, `ATLAS_HEFESTO_KEY`,
`ATLAS_HEFESTO_DEVICE_ID` y `ATLAS_HEFESTO_URL` opcional) se leen solo del
entorno del proceso Tauri. Sin ellas, `phase` es `unconfigured` y el frontal
oculta el indicador: no existe un Hefesto inerte.

`deviceId` se deriva del certificado mTLS, nunca del cuerpo del cliente. `instanceId`
identifica un arranque; `generation` invalida trabajo anterior; `submissionId` evita
duplicar envíos y `invocationId` identifica la entrega/resultado. `requestId`,
`sessionId` y `taskId` sirven para correlación, no conceden permisos.

El journal conserva `pending`, `dispatched`, `succeeded`, `cancelled`, `expired` y
`unknown`. Reiniciar cancela lo pendiente e invalida instancias; un despachado sin
resultado queda desconocido, sin reintento automático. El protocolo no promete
exactly-once para efectos externos ni sustituye el consentimiento por acción.

### Privacidad

El contenido de una escritura nunca se escribe en logs ni en el journal en disco:
solo llega al archivo destino al ejecutarse. El ejecutor del dispositivo conserva
en memoria las invocaciones completadas —cada una puede incluir hasta 16 KiB de
contenido de escritura— para deduplicar entregas y responder a un ACK perdido;
ese contenido muere con el proceso, y el badge de estado solo transporta texto de
eco, nunca contenido de archivos.
