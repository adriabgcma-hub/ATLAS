# Especificación: delegación por intención a Hefesto (entrega 3)

- Fecha: 2026-10-29
- Estado: diseño aprobado por el usuario; pendiente de revisión final y plan de implementación
- Fuente: `docs/hefesto-plan.md` §8 (entrega 3) y decisiones de la sesión de diseño

## 1. Contexto y objetivo

Hefesto es el especialista de Mac de Atlas: el relay (cerebro) piensa y coordina;
el dispositivo en el Mac ejecuta. Hoy Hefesto ya vive en la app (indicador, kill
switch, arranque listo, suspensión persistente) y el relé dispone de un listener
mTLS con journal durable, pero **el chat no le delega nada**: el listener solo hace
eco de pruebas.

Esta entrega cierra el bucle: cuando el usuario pide algo que es trabajo de
Hefesto, Atlas lo detecta **por intención** (el LLM decide con la herramienta
disponible), delega el encargo y **no bloquea la conversación**. Cuando Hefesto
termina, el resultado aparece en el chat como un mensaje propio, etiquetado
HEFESTO.

Primera rodaja: **solo el eco** (`hefesto.echo`). Nada de acciones reales sobre
el Mac todavía; el mecanismo de delegación, seguimiento y presentación es lo que
se valida.

## 2. Requisitos

Identidad y UX (aprobados por el usuario):

- Atlas responde un mensaje interino del estilo «se lo paso a Hefesto» y cierra
  su turno; la conversación queda libre de inmediato (no bloqueante).
- El resultado llega después como mensaje continuador con la etiqueta **HEFESTO**
  y estilo violeta, claramente distinto de los mensajes de Atlas.
- El usuario no distingue canales: la misma experiencia en local y en Telegram.
- Solo se delega si Hefesto está disponible; si no, Atlas lo dice y no delega.

Comportamiento:

- Delegación por intención: el LLM decide cuándo llamar a la herramienta
  (sin reglas de texto frágiles).
- Cancelación del turno no aborta el dispositivo ni deja continuadores colgados.
- El eco es una operación trivial: sin confirmación (matriz de
  `docs/hefesto-plan.md`).
- Recuperación tras reinicio del relay: una delegación en vuelo se resuelve
  como `unknown` y se publica su continuador.
- Contratos tipados espejo (docs/contracts.md, contracts.rs, contracts.ts).
- Ningún proceso nuevo ni autoarranque en el Mac: el seguimiento vive en el
  worker/hilos del relay, que ya existen.

## 3. Alternativas consideradas

**A (elegida) — delegación asíncrona con hilo de seguimiento.**
`hefesto_echo` encola el encargo en el journal y vuelve al instante; Atlas emite
el interino; un hilo dedicado sondea el resultado y publica el continuador.

- A favor: no bloquea, un solo turno a la vez (el worker conserva su invariante),
  encaja con el journal durable ya existente.
- En contra: dos mensajes por delegación (interino + resultado) y un hilo más
  que vigilar.

**B — pool de hilos con herramienta bloqueante.**
El tool espera el resultado dentro del turno y solo hay un mensaje final.

- Rechazada: dos turnos concurrentes sobre la misma sesión (estado, eventos,
  confirmaciones) introducen carreras que hoy no existen; el worker de un solo
  hilo es una invariante del relay.

**C — bloqueante en el worker actual.**
El turno se congela hasta 15 s esperando al dispositivo.

- Rechazada: viola el requisito no bloqueante y, si el dispositivo no responde,
  el chat queda mudo hasta el TTL.

## 4. Diseño elegido

### 4.1 Arquitectura general

```
usuario → relay (Brain) ── thinking ──→ LLM (Groq)
                        ↑                    │ llama a hefesto_echo
                        │                    ↓
              mensaje interino        journal.submit (controlador en proceso)
              + completed             │
                                      ↓
                hilo de seguimiento ──→ outcome (sondeo hasta terminal)
                                      ↓
              message_appended {agent: hefesto, taskId} → UI violeta
              Telegram: envío al chat_id de origen
```

### 4.2 El relay como controlador en proceso

`hefesto_listener()` (server/atlas_relay.py) hoy crea el `Journal` y lo descarta.
Pasa a **entregárselo al `Brain`** (el contextmanager `yield` el journal). El
`Brain`:

- deriva el `device_id` del **único** dispositivo de `peers.json`
  (más de uno → `fail_config`: la delegación no es ambigua);
- actúa como par sintético `Peer("controller", device_id)`, igual que la consola
  `hefesto-control` usa hoy: misma RBAC, mismas reglas del protocolo;
- sin listener activo (sin `HEFESTO_*` en el entorno), la herramienta no se
  ofrece y no hay hilo de seguimiento.

### 4.3 Protocolo v1: operación `outcome`

El controlador hoy puede encolar (`submit`) pero **no puede leer resultados**;
solo el dispositivo ve sus invocaciones. Se añade al protocolo v1:

```
POST /v1/hefesto/outcome
{ "invocationId": "…" }
→ 200 { "protocolVersion": 1, "invocationId": "…", "state": "…",
        "outputText": "…" | null }
```

- RBAC: solo rol `controller`, igual que `submit`.
- Validación: mismo tratamiento que el resto de operaciones (`_Rejected` con
  códigos estables; invocación inexistente → 404 `invocation_not_found`).
- Sin efectos secundarios: lectura pura del journal.

### 4.4 Herramienta `hefesto_echo`

- `TOOL_DEFS` pasa a construirse dinámicamente: `hefesto_echo` solo se incluye
  si el listener está activo.
- El tool comprueba disponibilidad antes de encolar (estado del dispositivo:
  suspendido/offline/ocupado). Si no está disponible, devuelve un estado
  `unavailable` estructurado y el LLM lo comunica sin delegar; el turno acaba
  con ese mensaje y no hay continuador.
- Si está disponible: `submit` al journal con `submissionId` idempotente y
  devuelve inmediatamente el estado `pending`; Atlas emite su interino
  («se lo paso a Hefesto») y `completed`.
- El eco es trivial (sin confirmación); operaciones futuras no triviales
  pasarán por la matriz de `docs/hefesto-plan.md` (confirmación individual).

### 4.5 Flujo de seguimiento

1. Al encolar, el `Brain` registra la delegación pendiente (invocationId,
   taskId, sessionId, requestId del turno, chat_id de Telegram si aplica).
2. Un hilo dedicado (uno solo; nace y muere con el worker del relay) sondea
   `outcome` con backoff corto hasta estado terminal: `succeeded`/`expired`/
   `unknown`/`cancelled`.
3. Al terminar publica `conversation.message_appended` con:
   - `role: 'assistant'`, `agent: 'hefesto'`,
   - **requestId nuevo** (el reducer desduplica por `requestId:role`: el
     continuador jamás colisiona con el mensaje interino),
   - `taskId` de correlación con la delegación,
   - contenido según estado: eco devuelto en `succeeded`, mensaje claro de
     caducidad/fallo en el resto.
4. Telegram: el seguimiento se envía al `chat_id` de origen del turno.

### 4.6 Cancelación

`/v1/cancel` marca la delegación como cancelada en el registro del Brain:

- no se publica continuador (el interino ya explicó la delegación);
- **no** se aborta el dispositivo (el eco es inofensivo y el aborto sigue siendo
  exclusivo del kill switch/control);
- el sondeo deja de vigilar esa invocación.

### 4.7 Recuperación tras reinicio

Al arrancar, el `Brain` resuelve las delegaciones persistidas no terminales como
`unknown` (el worker anterior murió) y publica el continuador correspondiente.
El journal del dispositivo ya cubre su lado (estados `cancelled`/`unknown` al
arrancar).

## 5. Contratos

`MessageAppended` gana dos campos opcionales (espejo en los tres sitios):

```ts
export interface MessageAppended extends Correlation {
  type: 'conversation.message_appended'
  role: MessageRole
  content: string
  source: Source
  /** Nuevo: agente emisor del mensaje; ausente = Atlas. */
  agent?: 'atlas' | 'hefesto'
  /** Nuevo: correlación con la delegación (taskId del protocolo Hefesto). */
  taskId?: string
}
```

- Sin eventos de estado nuevos: un continuador no cambia `SessionState`.
- `ChatMessage` en el frontal gana `agent?: 'atlas' | 'hefesto'` (derivado del
  evento) para decidir etiqueta y estilo.
- Tabla del protocolo Hefesto en `docs/contracts.md`: nueva operación `outcome`
  (rol controller, lectura pura).

## 6. Errores y estados

| Caso | Comportamiento |
| --- | --- |
| Hefesto suspendido/offline/ocupado al delegar | Tool devuelve `unavailable`; Atlas lo dice; sin continuador |
| Dispositivo no responde (TTL 15 s) | `expired` → continuador explicando la caducidad |
| Relay reinicia con delegación en vuelo | `unknown` → continuador al arrancar |
| Usuario cancela el turno | Sin continuador; dispositivo intacto |
| Sin listener Hefesto | La herramienta no se ofrece al LLM |

Códigos estables vía `_error`/`AtlasError`; nada de secretos ni rutas en
mensajes.

## 7. Seguridad

- El controlador es el relay en proceso: sin credenciales nuevas, sin socket
  nuevo, sin procesos nuevos.
- `outcome` es lectura pura y exige rol controller (RBAC existente).
- El frontal sigue sin conocer claves ni el protocolo: solo recibe el mensaje
  tipado con `agent`/`taskId`.
- La matriz de `docs/hefesto-plan.md` no cambia: eco trivial sin confirmación;
  lo no trivial, confirmación individual.
- Métricas con `dlog` HEFESTO: latencia por delegación, sin contenido sensible.

## 8. UI

- `ConversationPanel`: el mensaje con `agent: 'hefesto'` muestra la etiqueta
  **HEFESTO** y estilo violeta (`App.css`), igual que hoy se distingue
  TÚ/ATLAS/TG · TÚ.
- El mensaje interino sigue siendo un mensaje normal de ATLAS.
- El efecto de escritura (reveal) se aplica también al continuador, con la
  duración acotada ya existente.

## 9. Pruebas

- **Journal (server/hefesto.py):** `outcome` solo para controller; validación de
  cuerpo; invocación inexistente → 404; lectura no muta estado.
- **Brain con journal falso:** disponible/ocupado/éxito/caducidad/cancelación/
  recuperación tras reinicio; idempotencia del continuador.
- **TS:** reducer (dedupe interino vs continuador, agent/taskId), panel
  (etiqueta HEFESTO violeta).
- **En vivo:** delegación real con eco (local y Telegram), cancelación a medias,
  suspensión con encargo en vuelo.
- Regresión: `cargo check --workspace`, `cargo test`, `npm run build`,
  `npm run lint`; despliegue del relay si toca `server/` (sí toca).

## 10. Archivos afectados

- `server/atlas_relay.py` — journal entregado al Brain, tool dinámico, hilo de
  seguimiento, cancelación, recuperación, dlog.
- `server/hefesto.py` — operación `outcome` + tests.
- `docs/contracts.md` — MessageAppended y tabla de protocolo.
- `crates/atlas-core/src/contracts.rs` — espejo Rust.
- `src/types/contracts.ts` — espejo TS.
- `src/state/conversation.ts` — `agent`/`taskId` en ChatMessage (dedupe intacto).
- `src/components/chat/ConversationPanel.tsx` — etiqueta HEFESTO.
- `src/App.css` — estilo violeta.
- `docs/security.md`, `docs/implementation-plan.md`, `docs/hefesto-plan.md`,
  `docs/implemented.md` — registro.
- `docs/superpowers/specs/2026-10-29-hefesto-delegacion-design.md` — este
  documento.

## 11. Fuera de alcance (esta rodaja)

- Acciones reales sobre el Mac (adaptadores de archivos, comandos, etc.):
  entran en entregas posteriores con su matriz de confirmaciones.
- Multi-dispositivo: `fail_config` si `peers.json` tiene más de un dispositivo.
- Atajo local Cmd+Shift+H y control desde Hub Panel (entrega 2 pendiente).
