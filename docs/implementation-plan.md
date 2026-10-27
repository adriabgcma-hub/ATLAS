# Plan de implementación — Fase 2 (alcance actual)

> Archivo acotado a la Fase 2 por decisión del usuario (2026-10-12).
> Las fases 1 y 3–6 del documento anterior se retiraron; se reintroducirán cuando se pidan.

## Objetivo

Atlas recibe mensajes por **2 canales de entrada** y los presenta en una única UI:

- **Canal local:** barra de input de la app (`InputBar`, clase `.input-bar`).
- **Canal Telegram:** bot propio que el servidor consulta por polling saliente.

El **servidor gestiona las peticiones**; el **frontend solo actúa y presenta UI**,
y **mantiene la cadencia visual** de las letras de energía (`spawnLetter`).

## Decisiones cerradas

1. **Cadencia visual:** la mantiene el frontend (`App.tsx`, `LETTER_CADENCE_MS`).
   El servidor nunca toca el pool de sprites ni el timing.
2. **Servidor:** Python, mínimo consumo, ejecutable en cualquier ordenador 24/7.
   Framework definitivo para IA después.
3. **Rol del servidor:** solo reenvía texto al núcleo. No decide permisos.
4. **Confirmaciones:** las da el usuario en la UI. Si hay que persistir permisos,
   `permissions.json` local (ignorado por Git).
5. **Simultaneidad:** cola FIFO. Dos peticiones a la vez se encolan, no se pierden.
6. **Red:** el servidor solo acepta conexiones de la misma red (bind LAN + allowlist).
   Telegram se consulta por **polling saliente (`getUpdates`)**, nunca webhook
   entrante: así el servidor no se expone a internet.
7. **LLM (registrado, no implementado):** 3 API keys rotativas de Groq (free, ya en
   `.env`), modelo `gpt-oss-120b`. La integración del proveedor queda pendiente de
   petición explícita (ver `construction-rules.md`: nada de proveedores ni prompts
   sin petición explícita).
8. **Claves Tavily en `.env`:** existen pero sin uso en código (`rg` sin resultados).
   Fuera de alcance de esta fase.

## Estado actual (revisado 2026-10-26)

- [x] `InputBar` abajo a la izquierda + letras al orbe vía `ScenePort`.
- [x] Contratos TS↔núcleo (`Source`, `MessageAppended`; `contracts.md` ampliado).
- [x] Servidor relay Python (polling saliente, FIFO, solo LAN).
- [x] Núcleo de sesiones + IPC (`conversation_send/cancel`, `session_get`,
      `relay_drain`; eventos `session.state_changed` y
      `conversation.message_appended`; errores `AtlasError`).
- [x] Frontend suscrito a eventos (misma cadencia para mensajes entrantes,
      cancelación desde la barra).
- [x] Pruebas (`npm run build`, `npm run lint`, `npm run test`,
      `cargo test -p atlas-core`, `cargo check --workspace`, humo del relay).
- [x] Verificación visual manual (cubierta por el uso continuado y por las
      verificaciones posteriores de F3–F5).
- [x] Commit publicado: `Fase 2: canales, relay y sesiones`.

## Fase 3 — Cerebro en Python + panel (cerrada)

> El LLM vive en el relay 24/7 con la app cerrada (decisión del usuario).
> Rust adelgaza a transporte; el panel muestra mensajes del LLM vengan de
> donde vengan.

- [x] Relay cerebro: sesiones persistidas, Groq rotativo, worker FIFO único,
      respuesta directa a Telegram, cola de eventos (`/v1/events|send|cancel|session`).
- [x] Núcleo Rust fino (contratos + HTTP): `session.rs` y `provider.rs`
      eliminados (un solo cerebro, sin duplicar sesiones ni claves).
- [x] Panel de conversación (usuario + ATLAS, marca de canal TG).
- [x] Pruebas (`cargo test`, `cargo check`, build, lint, vitest, cerebro live
      con Groq real). Incidencia: Groq 403 al UA de urllib → UA propio.
- [x] Fix replay: el cursor se reinicia en cada carga y re-jugaba toda la cola
      (letras sin fin). Ahora `/v1/session` devuelve el cursor vivo, el frontal
      hidrata sin letras y solo vuelan los mensajes nuevos (vistos por
      `requestId:rol`).
- [ ] Verificación Telegram real con app cerrada.
- [x] Commit publicado: `Fase 3: cerebro en Python y panel` (`307aa7a`).

## Fase 4 — Tools de calendario (cerrada salvo re-verificación)

> `calendar_read` (todos) + `calendar_write` (solo editables, con
> confirmación). Bucle multi-turno en el cerebro, UI de confirmación.

- [x] CalDAV escritura (create/update/delete, UID, escape RFC 5545, parche textual).
- [x] Registro, ejecutor con allowlist servidora, confirmaciones con caducidad,
      `/v1/confirm`, `permission_respond`, panel ámbar en UI.
- [x] Verificado live: lectura, confirm→deny, confirm→allow con rechazo en
      Familia (no editable), escritura real en Deberes (PUT aceptado).
- [x] Fix panel fantasma: sin reset al cambiar de sesión se acumulaban
      historiales (`seq` persistido en disco + reset por `sessionId` en reducer).
- [x] Comandos `/clear` (sesión nueva real) y `/help` (sin gastar Groq).
- [x] Comandos locales ampliados y desplegados (`d37ef88`): alias
      `/limpiar`, `/ayuda` y `/registro`; `/log` con las 5 últimas interacciones
      (id, mensaje y tokens por petición) y consumo total; los comandos
      desconocidos responden localmente y el chat respeta los saltos de línea.
      La entrega de eventos a la app se arregló en `93716eb` (canales Tauri
      con guiones e IPC asíncrono).
- [ ] Re-verificar lectura/borrado del evento de prueba al despejar throttle.
- [x] Commit publicado: `Fase 4: tools de calendario` (`a191559`).

## Presentación del chat (2026-10-27)

- [x] Markdown real en las respuestas del LLM (`703b135`): `react-markdown` +
      GFM + saltos de línea, HTML crudo desactivado, tablas con scroll
      horizontal y cabeceras sin partir; los mensajes del usuario siguen en
      texto plano.
- [x] Streaming adaptativo: duración según longitud con techo de 6 s y avance
      por tiempo transcurrido (sobrevive al throttling de pestañas en segundo
      plano).
- [x] Cursor en línea con el texto Markdown: se inserta en el último bloque
      renderizado, no como hermano posterior al párrafo; eliminado su margen
      superior. En código se coloca antes del salto final añadido por Markdown.
      Sin cambios en la velocidad de revelado ni en los mensajes del usuario.
      Verificado en desarrollo (2026-10-27): build, 31/31 tests (16 nuevos),
      lint de los componentes del chat y `cargo check --workspace` correctos.
      Geometría DOM comprobada en navegador: cursor junto al último carácter
      en 11 casos (párrafos, saltos, títulos, listas, citas, código y tablas),
      sin cursor al terminar. `ConversationPanel` real con texto de prueba
      completa 696 caracteres en ~6,18 s incluso en pestaña oculta.
      Consola sin errores; captura no disponible por navegador integrado oculto.
      Lint global mantiene los 3 errores previos de Hub Panel.
      Vite cerrado por PID al terminar: puerto 5173 libre, sin procesos Atlas/dev
      activos; `launchctl list` conserva Hub Panel registrado pero sin PID.
      Commit `446ecb4`; bundle firmado e instalado en `/Applications/Atlas.app`.
- [ ] Tablas en ventana aparte (`Atlas · Tabla`) con detección + botón
      "Abrir en ventana" y exportar CSV a `~/Documents/Atlas/tablas/`
      (diseñado; pendiente de implementar).

## Variables de entorno (nombres; valores solo en `.env` local, nunca en Git)

- `GROQ_API_KEY_1/2/3` — rotación Groq (en uso por el cerebro).
- `TELEGRAM_BOT_TOKEN` — token de @BotFather.
- `TELEGRAM_ALLOWED_USER_IDS` — IDs de Telegram autorizados (rechazar el resto).

## Regla de avance

Cada pieza se considera terminada cuando está implementada, probada, documentada
y asociada a un commit publicado en GitHub con permiso explícito del usuario.
