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
- [x] Comando `/cancel` (`/cancela`) como comando local (2026-11-06): cancela
      la petición en vuelo vía `cancel_all()` y responde al instante sin gastar
      Groq; cubierto por 2 pruebas en `test_hefesto_integration.py`.
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
- [x] Órbitas por fase del turno (2026-10-30, `f4c2ddc`…`e0eb90c`): el relay
      publica `executing` con el toolId y la órbita de la petición se
      sustituye por la fase real — Hefesto en violeta («Leyendo archivo»,
      «Listando carpeta», «Encargo»), otras herramientas en azul y «Atlas ·
      Redactando respuesta» al responder. La primera versión dependía del
      render y el drenado por lotes la ocultaba: `phaseOrbits` crea la órbita
      al procesar cada evento y el drenado baja a 1 s (`1899526`). El cliente
      de Hefesto reintenta la resolución mDNS y construye TLS fuera del
      arranque (`e0eb90c`). Verificado en vivo: el flujo del relay emite
      `executing 'hefesto_read_file' → responding → completed` y la órbita
      violeta aparece tras permitir la lectura. Pruebas: 51 frontend, 150
      Python, `cargo test --workspace`, clippy y fmt (los 3 errores de lint de
      Hub Panel siguen siendo previos).
- [x] Tablas en ventana aparte (2026-11-03, `3cfe4da`): cada tabla GFM muestra
      `Abrir en ventana` dentro de Tauri; el payload viaja por IPC a una ventana
      `table-N` (`table.html`, `Atlas · Tabla`, 920×640) que lo guarda en memoria y
      ofrece `Exportar CSV` a `~/Documents/Atlas/tablas/` (BOM UTF-8, CRLF,
      comillas RFC 4180, nombre saneado y escritura atómica). Límites: título
      ≤120, 1–64 columnas, ≤2000 filas, celda ≤2000 caracteres, payload ≤512 KiB.
      Cubierto por pruebas Rust de validación/CSV y de frontend del botón, y
      verificado visualmente en la app por el usuario (2026-11-04).

## Hefesto — especialista del Mac (2026-10-27)

Plan completo y criterios: [Hefesto](./hefesto-plan.md). Commit, push y despliegue
de prueba autorizados por el usuario (2026-10-28).

- [x] Revisar historial y preservar la base publicada (`49c8cf2`).
- [x] Sustituir el borrador por el plan revisado: autenticación, identidad por
      invocación, disponibilidad real, deduplicación, privacidad y parada.
- [x] Base v1 local con mTLS, roles, leases, control y eco sin efectos; pruebas
      Rust–Python correctas. Revisión final cerrada (2026-10-28): registro en dos
      fases (lease anclado al primer envío) y autorización TLS antes de HTTP con
      plazo absoluto por conexión.
- [x] Desplegar y activar el listener de prueba en atlas-hub (8767, solo LAN,
      PKI de prueba fuera del repo) y validar el eco completo desde el Mac.
      `hefesto-echo` resuelve IPv4 (el link-local IPv6 de mDNS no escucha y
      agotaba el timeout en cada petición) y renueva la lease con `heartbeat`.
- [x] Primer corte de la integración en la app (2026-10-28): bucle nativo único
      (`atlas-mac::Runtime`) con heartbeat, sondeo de eco, aborto local con reintento
      acotado y reconexión con instancia nueva; comandos `hefesto_status/resume/abort`,
      evento `hefesto-state-changed` e indicador violeta con Activar/Suspender y último
      eco. Credenciales por entorno o Application Support; sin ellas no hay hilo
      ni indicador.
- [x] Arranque listo y suspensión persistente (2026-10-29): al abrir, el dispositivo
      se activa solo si no hay suspensión pendiente (Hefesto queda disponible por
      intención, no por un interruptor); suspender —local, Hub Panel o remoto— deja
      `hefesto-suspended` en el directorio de datos de la app y exige Activar a mano,
      también tras reiniciar. Probado en vivo con el relay real.
- [x] Cerrar la entrega 2 (2026-10-30): atajo local `cmd+shift+a` con lógica pura
      testeada, tarjeta Hefesto en el Hub Panel con suspensión remota
      (`GET /v1/hefesto/status` + `POST /v1/hefesto/suspend` en el relay) y matriz
      completa en vivo (salida normal, crash, pérdida de red, suspensión remota y
      local, reinicio con encargo en vuelo). Commits `1ade92e`…`cc92ca1`, desplegado.
- [x] Delegación no bloqueante, identidad visual y recuperación de tareas
      (2026-10-29, entrega 3): operación `outcome` de lectura, herramienta
      `hefesto_echo` con catálogo dinámico, continuador etiquetado HEFESTO con
      `agent`/`taskId` en los contratos, persistencia y recuperación tras reinicio.
      Desplegada en atlas-hub y probada en vivo (local y Telegram).
- [x] Lectura segura de archivos (2026-10-30, puerta 4, corte 1): protocolo v2 con
      `hefesto.list_dir`/`hefesto.read_file` sobre raíces del usuario (contención
      canónica, 16 KiB/100 entradas, binarios rechazados), consentimiento por lectura
      y resultado síncrono al modelo (la respuesta la redacta Atlas). Desplegado y
      probado en vivo (tarjeta, `LUNA-42`, fuera de raíces rechazado). Commits
      `3a4fd1a`…`295da62`.
- [x] Escritura segura de archivos (2026-10-30, puerta 4, corte 2): protocolo v3 con
      `hefesto.write_file`/`hefesto.rename`/`hefesto.file_info`, creación sin pisar
      (`hard_link`), sobrescritura con hash + `rename` atómico conservando el modo,
      movimiento sin pisar en el mismo volumen, contenido nunca persistido y tarjeta
      única «Crear/Sobrescribir/Mover» con el hash del plan. Probado en vivo (crear,
      sobrescribir, hash obsoleto rechazado sin pisar, mover, `/etc` fuera de raíces)
      y con visibilidad de Hefesto: burbuja violeta HEFESTO tras cada escritura o
      movimiento y tarjeta etiquetada HEFESTO. Commits `cf1f3ab`…`e1a4cee` más el de
      la burbuja/tarjeta, desplegado.
- [x] Control de apps (2026-11-01, puerta 7, corte 1): protocolo v4 con **una sola**
      herramienta `hefesto_apps` (`open`/`close`/`list`, trivial por adaptador y sin
      tarjeta), resolución híbrida de `NSWorkspace.runningApplications` + mapa
      cacheado de `Info.plist`, cierre **solo suave**, lista cortada a ≤30 y burbuja
      violeta HEFESTO solo en el éxito de abrir o cerrar; el puente AppKit vive en
      `src-tauri` y se reenvía al hilo principal por el trait inyectado `AppBridge`,
      sin API privada. Probado en vivo (abrir y cerrar TextEdit, lista real del Mac,
      app inexistente rechazada sin abrir). Commits `c17d993`…`355fa2a`, relay
      desplegado y bundle instalado en `/Applications/Atlas.app`.
- [x] Teclado sintético (2026-11-01, puerta 7, corte 2): protocolo v5 con **una sola**
      herramienta `hefesto_keyboard` (`type`/`combo`) y **tarjeta siempre**; solo la
      app en primer plano, texto ≤4096 B sin controles, atajos con al menos un
      modificador y teclas cerradas. El puente usa eventos Unicode de CGEvent en el
      hilo principal, pide el permiso de Accesibilidad una vez por arranque y respeta
      la entrada segura. El texto no se persiste, no se registra en el log ni genera
      burbuja (la tarjeta con el texto recortado es el consentimiento). Probado en
      vivo: tecleo verificado en un documento con fichero, `cmd+s` y rechazo por
      entrada segura. Commits `2ef02eb`…`24ae479`, relay desplegado y bundle
      instalado en `/Applications/Atlas.app`.
- [x] Multimedia (2026-11-01, puerta 5): protocolo v6 con **una sola** herramienta
      `hefesto_media` (volumen, brillo, silencio y reproducción) por teclas de
      sistema, `count` 1–10 solo para volumen/brillo y **tarjeta siempre**; sin
      lectura de estado, sin subprocess y sin permisos nuevos. El puente usa
      eventos system-defined de AppKit al hilo principal y reutiliza el permiso de
      Accesibilidad y la guarda de entrada segura del teclado. Verificado en vivo
      subir volumen dos pasos (0→12, sin silencio) y el rechazo estable con
      entrada segura; la matriz de validación dio `bad_count`/`bad_action`/
      `bad_argument`. Con la pantalla desbloqueada se completó la matriz (bajar
      volumen, silenciar dos veces, brillo y play/pausa con Music). Commits
      `1424ec2`…`6266fad`, relay desplegado y bundle instalado en
      `/Applications/Atlas.app`.
- [x] Ventanas (2026-11-01/02, puerta 7, corte 3): protocolo v7 con **una sola**
      herramienta `hefesto_windows` (`list`, `move`, `resize`, `focus`): `list` sin
      tarjeta y cada efecto con exactamente una, sin burbuja. Resolución de app y
      ventana (índice 1–99, título exacto/contiene/ambiguo) y validación estable en
      `crates/atlas-mac/src/windows.rs`; puente con **Accessibility pública**
      (`AXUIElement`) en `src-tauri`, timeout de 2 s por app y ventana, posición
      acotada a las pantallas y foco verificado de verdad con `frontmostApplication`
      (el defecto que devolvía éxito sin activar se corrigió en `f31a76a`). Journal
      v7 con claves cerradas por acción; el relay omite los títulos de ventana en el
      log. Probado en vivo (listar, mover, redimensionar, traer al frente desde otra
      app, ambigüedad y matriz de validación por mTLS). Commits `85c5b14`…`f31a76a`,
      relay desplegado y bundle instalado en `/Applications/Atlas.app`.
- [x] Terminal de consulta (2026-11-02/03, puerta 6, corte 1): protocolo v8 con
      **una sola** herramienta `hefesto_system` (`os`, `disk`, `uptime`, `memory`),
      sin tarjeta ni burbuja. El dispositivo solo elige entre adaptadores cerrados
      con binario y argumentos fijos (`sw_vers`, `df -kH /`, `uptime`, `vm_stat`),
      sin shell, entorno limpio, grupo de procesos propio, timeout de 2 s con
      `SIGKILL` del grupo y tope de 16 KiB. Journal v8 con `action` cerrada y
      validación estable; el relay omite el contenido del resultado en el log.
      Probado en vivo por HTTP (`os` → macOS 27.0.1 y `disk` → 42 GB, sin tarjeta
      ni burbuja) y con las pruebas de journal para acciones y claves inválidas.
      Commit `c440380`, relay v8 desplegado y bundle v8 instalado en
      `/Applications/Atlas.app`.
- [x] Terminal general (2026-11-03, puerta 6, corte 2): protocolo v9 con **una
      sola** herramienta `hefesto_terminal` (`{program, args}`), argv directo sin
      shell y **tarjeta por invocación** con riesgo `destructive`. Rutas absolutas
      solo en `/usr/bin`, `/bin`, `/usr/sbin`, `/sbin`, `/usr/local/bin` y
      `/opt/homebrew/bin`; intérpretes y elevación denegados por nombre; ≤32
      argumentos y ≤4096 B de comando. Entorno limpio, grupo propio, timeout de
      10 s con `SIGKILL` del grupo y 24 KiB por tubería. Journal v9 con el comando
      redactado en disco (`commandBytes`/`commandSha256`) y `unknown` tras
      reinicio. Probado en vivo (`/bin/echo hola` → tarjeta → `hola`).
      Commit `3cfe4da`.
- [x] Capturas con OCR local (2026-11-03, puerta 7, corte 4): protocolo v9 con
      **una sola** herramienta `hefesto_capture` (`{target: screen}`), tarjeta por
      invocación con riesgo `external`. TCC de Grabación de pantalla comprobado y
      solicitado como mucho una vez por proceso; PNG en
      `~/Documents/Atlas/capturas/`; OCR con Apple Vision (`Accurate`,
      `es-ES`/`en-US`); timeouts de 10 s; solo vuelve el texto, nunca la imagen ni
      la ruta. Probado en vivo hasta el rechazo estable por TCC pendiente; la ruta
      OCR real queda pendiente de conceder el permiso y reiniciar Atlas.
      Commit `3cfe4da`.
- [x] Resolución de apps por nombre localizado (2026-11-04): el catálogo lee
      `InfoPlist.loctable` e `InfoPlist.strings` de los idiomas del usuario y
      guarda alias; la comparación ignora espacios, acentos y puntuación. Probado
      en vivo: «abre buscar» y «abre find my» abren «Buscar» (`FindMy`). El
      arranque normal del bundle también registra; el «offline» anterior era una
      instancia vieja de `target/release` reteniendo la lease.
- [x] Fechas de agenda con zona y día semanal (2026-11-06): `calendar_read`
      conserva TZID/`VALUE=DATE`, resuelve a `RELAY_TIMEZONE`, calcula la semana
      siguiente (lunes→lunes) en el servidor y formatea con día de la semana
      (`lunes 9 nov 10:30` / `jueves 12 nov (todo el día)`); añade
      `period: "next_week"`. 10 pruebas offline (regresión 2026-11-06, semana
      correcta 9–15); Python 596. Commit `71b8d18`, relay desplegado.
- [ ] Pendiente: captura real con OCR tras conceder el TCC de Grabación de
      pantalla.

La API HTTP actual no puede lanzar Hefesto; el dispositivo se inicia dentro de la
app Tauri al abrirla, leyendo sus credenciales del entorno o de
`~/Library/Application Support/com.atlas.next/hefesto/`, y hoy cubre el eco, la lectura
y escritura dentro de las raíces autorizadas, el control de las apps instaladas,
el teclado sintético con consentimiento, el control multimedia por teclas de
sistema con tarjeta, la gestión de ventanas por Accessibility, las consultas de
sistema por adaptadores, el terminal general por argv directo con tarjeta y la
captura de pantalla con OCR local. La ventana de tablas está verificada en la app;
la captura real con TCC concedido queda como verificación final.

## Variables de entorno (nombres; valores solo en `.env` local, nunca en Git)

- `GROQ_API_KEY_1/2/3` — rotación Groq (en uso por el cerebro).
- `TELEGRAM_BOT_TOKEN` — token de @BotFather.
- `TELEGRAM_ALLOWED_USER_IDS` — IDs de Telegram autorizados (rechazar el resto).
- `ATLAS_HEFESTO_URL` (por defecto `https://atlas-hub.local:8767`), `ATLAS_HEFESTO_CA`,
  `ATLAS_HEFESTO_CERT`, `ATLAS_HEFESTO_KEY`, `ATLAS_HEFESTO_DEVICE_ID` — identidad mTLS
  del dispositivo Hefesto en la app; tienen prioridad sobre los ficheros de
  `~/Library/Application Support/com.atlas.next/hefesto/` (`ca.pem`, `device.pem`,
  `device-key.pem` 600, `device-id` y `url` opcional). Los ficheros viven fuera del repo.

## Regla de avance

Cada pieza se considera terminada cuando está implementada, probada, documentada
y asociada a un commit publicado en GitHub con permiso explícito del usuario.
