# Implementado

Historial de implementaciones publicadas en GitHub.

Cada entrada debe incluir el nombre exacto del commit que contiene el cambio.

## Historial

### Comando /log: últimos 3 registros en pantalla

- **Commit:** `Comando /log: muestra ultimos 3 registros del log sin llamar al LLM`
- **Fecha:** `2026-10-17`
- **Descripción:** comando `/log` que muestra los últimos 3 registros del log
  en el panel de chat sin llamar al LLM. Útil para debugging rápido.
- **Documentación relacionada:** `docs/relay-api.md`.

### Logging: log detallado + endpoints /v1/logs y /v1/stats

- **Commit:** `Logging: log detallado (respuestas, tokens, tools, interacciones) + endpoints /v1/logs y /v1/stats`
- **Fecha:** `2026-10-17`
- **Descripción:** log detallado a archivo (`temp/atlas-relay/relay.log`) con
  categorías RECEIVE, PROCESS, LLM, TOOL, COMPLETE, CANCEL, CONFIRM, TELEGRAM,
  LOCAL_REPLY. Tracking de tokens por modelo (prompt/completion/total/calls).
  Endpoints `GET /v1/logs?tail=N` y `GET /v1/stats`. Verificado live.
- **Documentación relacionada:** `docs/relay-api.md`.

### Fix CalDAV: evitar throttle de Apple

- **Commit:** `Fix CalDAV: evitar throttle de Apple (PROPFIND+GET en vez de REPORT) + ampliar barras de chat/input`
- **Fecha:** `2026-10-17`
- **Descripción:** el cliente CalDAV usaba `REPORT` (filtrado por fecha en servidor)
  que Apple throttled (403). Ahora usa `PROPFIND` (listar .ics) + `GET`
  (leer cada evento) con concurrencia (ThreadPoolExecutor, 8 workers) y
  filtrado de fecha en cliente. Caché de 120 s en el relay. Verificado live:
  lectura de 5 eventos en 11 s, escritura (crear/borrar) con confirmación UI.
  Barras de chat e input ampliadas a 400 px.
- **Documentación relacionada:** `docs/tools.md`, `docs/relay-api.md`.

### UI/Chat: streaming visual + limpieza de código muerto

- **Commit:** `UI/Chat: streaming visual + limpieza de codigo muerto del intento de streaming server`
- **Fecha:** `2026-10-16`
- **Descripción:** streaming simulado en el frontend (revelado progresivo
  letra a letra con cursor `▌`, cadencia 18 ms) sin depender del relay.
  Eliminado código muerto del intento de streaming server-side:
  `_push_stream`, tipo `StreamChunk`, acción `conversation.stream_chunk`,
  listener IPC y rama en el poll de dev. Contratos TS/Rust/docs coherentes.
- **Documentación relacionada:** `docs/contracts.md`, `docs/relay-api.md`.

### UI: anillo pensando + toggle del chat + panel suave

- **Commit:** `UI: anillo pensando + toggle del chat + panel suave`
- **Fecha:** `2026-10-16`
- **Descripción:** barra de entrada compacta (320px) con indicador orbital
  de pensamiento y toggle del panel apilados en el mismo hueco con fundido
  cruzado (sin desmontar nodos ni reiniciar el giro). Animaciones del panel
  con blur y easing (950ms aparecer, 1250ms desaparecer). Burbuja de
  órbita con estilo frosted glass. Colores fijos del agente (sin paleta
  multi-agente). Verificado: build, lint, test (13/13).
- **Documentación relacionada:** `docs/implementation-plan.md`.

### Fase 4: tools de calendario

- **Commit:** `Fase 4: tools de calendario`
- **Fecha:** `2026-10-16`
- **Descripción:** `calendar_read` (todos, auto) + `calendar_write` (solo
  Exámenes/Celebraciones/Horario/Deberes, siempre con confirmación en UI).
  Bucle multi-turno con function calling, allowlist servidora (ni el permiso
  del usuario la salta), confirmaciones con caducidad, panel ámbar.
  Verificado live: lectura, deny, allow con rechazo en Familia, escritura real
  en Deberes. Throttle de Apple intermitente durante las pruebas.
- **Documentación relacionada:** `docs/tools.md`, `docs/relay-api.md`,
  `docs/implementation-plan.md`.

### Fase 3: cerebro en Python + panel

- **Commit:** `Fase 3: cerebro en Python y panel`
- **Fecha:** `2026-10-16`
- **Descripción:** el cerebro se muda al relay Python 24/7 (sesiones
  persistidas, Groq con 3 claves rotativas, worker FIFO, respuesta directa a
  Telegram, cola de eventos). Rust adelgaza a contratos + HTTP (`session.rs` y
  `provider.rs` eliminados). Panel de conversación con respuestas del LLM.
  Verificación: cerebro live con Groq real; incidencia 403 por UA de urllib
  resuelta con UA propio; fix replay (cursor vivo en `/v1/session`, hidratación
  sin letras, vistos por `requestId:rol`).
- **Documentación relacionada:** `docs/architecture.md`,
  `docs/implementation-plan.md`, `docs/relay-api.md`.

### Revisión de estabilidad (2026-10-16)

- **Commit:** `Fase 3: cerebro en Python y panel`
- **Descripción:** relay: poda del conjunto de canceladas, backoff exponencial
  del poll (5s→60s), salida dura con token inválido, `health.events` honesto,
  timestamps en log. Frontal: tope de 200 mensajes, poda de timers al drenar.
  Rust: cero warnings, clippy limpio. Suite verde + ciclo Groq live.
- **Limitaciones conocidas:** cola de jobs sin cota (latencia crece si Groq va
  lento); redelivery de Telegram ante caída entre batches (duplicado posible);
  eventos en memoria (no sobreviven al reinicio; el historial sí).

### Fase 2: canales, relay y sesiones

- **Commit:** `Fase 2: canales, relay y sesiones`
- **Fecha:** `2026-10-12`
- **Descripción:** Atlas recibe mensajes por 2 canales (input local + Telegram)
  que convergen en el núcleo. `crates/atlas-core`: contratos espejo, sesiones
  identificadas/cancelables, stub local tras interfaz `Responder`, cliente HTTP
  mínimo del relay (solo std). IPC Tauri: `conversation_send/cancel`,
  `session_get`, `relay_drain`; eventos `session.state_changed` y
  `conversation.message_appended`; errores `AtlasError`. Relay Python
  (stdlib, polling saliente, FIFO, solo red local). Frontend suscrito a eventos
  con la misma cadencia visual en ambos canales y botón de cancelar.
  Verificación: `npm run build`, `npm run lint`, `npm run test` (10/10),
  `cargo test -p atlas-core` (15/15), `cargo check --workspace`, humo del relay.
- **Documentación relacionada:** `docs/contracts.md`, `docs/implementation-plan.md`,
  `docs/architecture.md`.

### Bot de Telegram creado y verificado

- **Commit:** `Fase 2: canales, relay y sesiones` (el token vive solo en
  `.env` local, nunca en Git).
- **Fecha:** `2026-10-12`
- **Descripción:** bot @ATLAS_Assister_BOT creado vía @BotFather y verificado con
  `getMe`. `TELEGRAM_ALLOWED_USER_IDS` fijado con 1 id. Modo polling saliente
  (`getUpdates`), sin webhook, según lo decidido en el plan.
- **Documentación relacionada:** `docs/implementation-plan.md`.

### Initial commit

- **Commit:** `Initial commit`
- **Descripción:** creación del repositorio nuevo con la base visual actual de Atlas y su configuración inicial.

### Anclar documentación y cerrar fase 1

- **Commit:** `Anclar documentación y cerrar fase 1`
- **Fecha:** `2026-10-12`
- **Descripción:** publica `AGENTS.md` y toda la documentación de Atlas (arquitectura, contratos, reglas de construcción, plan, implementado, seguridad, release y testing) y marca la Fase 1 — base visual estable — como completada. Verificación previa: `npm run build`, `npm run lint` y `cargo check --workspace` pasan sin errores.
- **Documentación relacionada:** `docs/index.md`, `docs/architecture.md`, `docs/contracts.md`, `docs/construction-rules.md`, `docs/implementation-plan.md`, `docs/implemented.md`, `docs/security.md`, `docs/release.md`, `docs/testing.md`.

## Plantilla para futuras implementaciones

### Nombre exacto del commit

- **Commit:** `Nombre exacto del commit`
- **Fecha:** `AAAA-MM-DD`
- **Descripción:** resumen breve de lo implementado.
- **Documentación relacionada:** enlaces a los documentos afectados.

