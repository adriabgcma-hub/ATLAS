# Implementado

Historial de implementaciones publicadas en GitHub.

Cada entrada debe incluir el nombre exacto del commit que contiene el cambio.

## Historial

### Chat: markdown formateado y streaming adaptativo

- **Commit:** `feat(chat): markdown formateado y streaming adaptativo`
- **Fecha:** `2026-10-27`
- **Descripción:** las respuestas del LLM dejan de mostrarse como markdown crudo. Se añaden `react-markdown` 10, `remark-gfm` 4 y `remark-breaks` 4, y `MarkdownMessage` (memoizado) renderiza títulos, listas, código y tablas GFM; el HTML crudo queda desactivado (react-markdown lo escapa), los enlaces abren en pestaña nueva y las tablas van envueltas en un contenedor con scroll. `ConversationPanel` renderiza solo los mensajes del assistant como markdown; los del usuario siguen en texto plano. Las tablas pasan a medir según su contenido (`width: auto; min-width: 100%`), con mínimo de 84 px por columna y cabeceras `white-space: nowrap`: en un chat de 296 px una tabla de 6 columnas ocupa 598 px y scrollea en horizontal en vez de partir los títulos letra a letra. El revelado progresivo calcula su duración con la longitud (`streamDuration` = longitud × 18 ms acotado entre 400 ms y 6 s) y avanza por tiempo transcurrido, no por tick, de modo que un mensaje de 3759 caracteres se completa en ~6 s (antes ~68 s) y una pestaña en segundo plano se pone al día al volver en lugar de alargar la espera. Verificado: `npm run build`, `npm test` (15/15) y eslint de los ficheros nuevos; en navegador, tabla inyectada de 6 columnas con 598/296 px y scroll, cabeceras en una línea, y streaming de 3759 caracteres medido en 4,18 s desde que se detectó el cursor (≈6 s desde el inicio). Bundle reconstruido con Apple Development, firma profunda válida (`codesign --verify --deep --strict`), binario embebe `index-ClN1lmhm.js` e `index-uDjekY4f.css`, instalado con `ditto` en `/Applications/Atlas.app` y abierto. Coste: el JS del frontal pasa de ~219 KB a 382,72 KB (119,20 KB gzip). El Vite de desarrollo se cerró por PID al terminar.
- **Documentación relacionada:** `docs/implementation-plan.md` (§ Presentación del chat).

### Chat: identidad estable de mensajes al rehidratar

- **Commit:** `fix(chat): identidad estable de mensajes al rehidratar`
- **Fecha:** `2026-10-27`
- **Descripción:** a veces el revelado progresivo del último mensaje se reiniciaba a mitad y volvía a imprimir todo el texto. Causa: el relay persistía la respuesta del assistant con un `requestId` nuevo (`new_id("req")`) mientras el evento `conversation.message_appended` llevaba el `requestId` de la interacción; la rehidratación cada 30 s sustituía el historial y cambiaba la identidad del último mensaje, y `ConversationPanel` lo tomaba por un mensaje nuevo (`revealed = 0`). Ahora el assistant persistido comparte `requestId` con la petición del usuario, tanto en respuestas normales como en comandos locales, y la hidratación de la misma sesión fusiona el snapshot con los mensajes recibidos en vivo en vez de reemplazarlos, de modo que un snapshot anterior al último evento ya no lo borra. Verificado: `npm test` 15/15 (dos pruebas nuevas para la fusión), `tsc -b` y lint; relay desplegado con `./scripts/deploy-relay.sh` (`/v1/health` OK) y comprobado que el usuario y el assistant de un `/help` comparten `requestId` (`req-4764…`); bundle firmado con Apple Development (firma profunda válida), `index-MpYmPoOi.js` embebido, reinstalado en `/Applications/Atlas.app` y app abierta.
- **Documentación relacionada:** `docs/relay-api.md` (§ `GET /v1/session`).

### App: eventos IPC con canales válidos y red fuera del hilo principal

- **Commit:** `fix(app): recibir eventos IPC y no bloquear el hilo principal`
- **Fecha:** `2026-10-27`
- **Descripción:** la app parecía no responder aunque el relay sí procesaba los mensajes. Causa doble: (1) Tauri 2 no admite puntos en nombres de evento, el frontend escuchaba `session.state_changed` / `conversation.message_appended` / `permission.confirmation_required` y el núcleo emitía los mismos nombres con guiones, de modo que React no recibía ningún evento y el orbe se quedaba en el estado `failed` anterior; (2) una petición `session_get` hacía E/S HTTP síncrona en el hilo principal (stack del hang de las 23:02: `run_invoke_handler → session_get → relay::http → poll`). El núcleo pasa a emitir y escuchar los canales `session-state-changed`, `conversation-message-appended` y `permission-confirmation-required` (el contrato con puntos sigue dentro del payload como `type`), y todos los comandos IPC (`conversation_send`, `conversation_cancel`, `permission_respond`, `session_get`, `relay_drain`) ejecutan la red con `spawn_blocking`. El cliente del relay ordena IPv4 antes que el IPv6 link-local que devuelve `atlas-hub.local` y limita cada intento a 2 s; el frontend re-hidrata al montar y cada 30 s, y el chat respeta los saltos de línea con `white-space: pre-wrap`. Verificado: `cargo check --workspace`, `cargo test --workspace` (8 + 1 + 2 pruebas OK), `npm run build`, `npm test` (13/13) y `npm run lint` (solo los 3 errores previos de `apps/panel`). Bundle reconstruido con Apple Development, firma profunda válida (`codesign --verify --deep --strict`) e instalado con `ditto` en `/Applications/Atlas.app`; el binario embebe `index-BKVKt3rr.js` / `index-BcGiiHem.css` y el usuario confirmó que responde en release.
- **Documentación relacionada:** `docs/contracts.md` (§ Canales de eventos IPC), `docs/implementation-plan.md`.

### Relay: comandos locales con alias y /log con tokens

- **Commit:** `feat(relay): /log con tokens y comandos locales con alias`
- **Fecha:** `2026-10-27`
- **Descripción:** los comandos de chat se resuelven localmente sin llamar a Groq y valen por la app y Telegram. Se reconoce el primer token sin distinguir mayúsculas; cualquier texto que empiece por `/` se trata como comando (nunca llega al modelo) y uno desconocido responde con el aviso y la ayuda. `/clear` (`/limpiar`) reinicia la conversación, `/help` (`/ayuda`) lista comandos y explica el flujo de confirmaciones y calendarios editables, y `/log` (`/registro`) muestra las 5 últimas interacciones de la más reciente a la más antigua con `requestId`, canal, hora local, mensaje, tokens (`prompt`/`completion`/`total`/llamadas) y resultado, seguidas del consumo total, desglose por modelo, sesión activa y estado de cola/vuelo. El uso por petición vive en memoria y se poda (no se persiste), y las respuestas conservan los saltos de línea en ambos canales. Verificado tras `./scripts/deploy-relay.sh`: push y pull `8fa8316..93716eb`, servicio `atlas-relay` `active` desde las 18:04:37, `/v1/health` OK (`telegram: enabled`, `llm: groq`, `queue: 0`) y `sha256` de `server/atlas_relay.py` idéntico en el Mac y en `/opt/atlas/repo`.
- **Documentación relacionada:** `docs/relay-api.md` (§ Comandos de chat), `docs/implementation-plan.md`.

### Hub Panel: ventana bajo demanda y ~16 MB en reposo

- **Commit:** `perf(panel-app): ventana bajo demanda y 16 MB en reposo`
- **Fecha:** `2026-10-26`
- **Descripción:** la app de barra de menús deja de mantener el `WKWebView` vivo en reposo. `tauri.conf.json` pasa a `"windows": []` y la ventana la crea `create_panel_window()` dentro de `show_panel()` (1100×820, mínimo 720×520, centrada) cuando se abre el panel desde el icono, el menú, una notificación o el arranque de la app; al cerrar, `CloseRequested` la destruye (`window.destroy()`) y solo queda la bandeja. Para que no se quede detrás, `show_panel()` hace `show()`, `unminimize()` y `set_focus()` también cuando la ventana acaba de nacer. Destruir la última ventana dispara `RunEvent::ExitRequested`, así que `PanelState.closing_window` permite `api.prevent_exit()` solo en Cmd+W (cerrar oculta); "Salir" y Cmd+Q siguen cerrando la app. Con la ventana cerrada, "Reiniciar relay…" marca `restart_requested` y la UI lo lee con el comando `pending_restart` (sin consumir, por el doble montaje de React StrictMode) y lo limpia con `clear_restart_request` al cancelar o terminar. Memoria medida con `footprint`: ~16 MB en reposo (antes ~110 MB) y ~31 MB app + ~64 MB `WebKit WebContent` con la ventana abierta. Verificado: `cargo check --workspace`, `cargo test -p atlas-panel`, `cargo fmt -p atlas-panel -- --check`, bundle firmado con Apple Development (14,63 MiB, firma profunda válida) instalado con `ditto` en `~/Applications`, y prueba del usuario de abrir/cerrar, foco al abrir y flujo de reinicio (incluida la cancelación). Build web regenerado (`index-kGpyvxw7.js`, `index-BcTWnyd2.css`) y desplegado en `atlas-hub`.
- **Documentación relacionada:** `AGENTS.md` (§ App Hub Panel: ventana y ciclo de la ventana), `docs/panel-app-plan.md` (§7 "Ventana bajo demanda y memoria", §6, §8), `docs/architecture.md` (§ Hub Panel: ventana).

### Relay: keep-alive HTTP/1.1 para bajar la latencia del panel

- **Commit:** `perf(relay): keep-alive HTTP/1.1 para bajar la latencia del panel`
- **Fecha:** `2026-10-26`
- **Descripción:** el relay respondía con HTTP/1.0 y `Connection: close`, de modo que cada sondeo del panel abría un TCP nuevo y pagaba el intento previo de IPv6 link-local del nombre `atlas-hub.local` (~220 ms) antes de caer a IPv4 (~8 ms). El `Handler` pasa a `protocol_version = "HTTP/1.1"` con `timeout = 15` para cerrar conexiones ociosas; el servidor ya era `ThreadingHTTPServer`, así que las conexiones vivas no bloquean a otros clientes, y todas las respuestas llevan `Content-Length` (el `204` de CORS no tiene cuerpo). Verificado tras desplegar `12bff7e`: `curl` muestra `HTTP/1.1 200 OK` y `Re-using existing connection`; 10 peticiones seguidas a `/v1/health` dan 0,23 s la primera (0,218 s de conexión) y ~0,010 s las demás; en caliente `/v1/system` 0,011 s (0,051 s la primera si regenera el snapshot), `/v1/session` 0,012 s, `/v1/stats` 0,010 s y `/v1/logs` 0,015 s. `/v1/health` OK y watchdog sin cambios.
- **Documentación relacionada:** `docs/relay.md` (§6 frescura y transporte), `docs/relay-api.md` (transporte HTTP).

### Panel: refresco cada 2 s y sin emojis en UI y bandeja

- **Commit:** `perf(panel): refresco cada 2 s y sin emojis en UI y bandeja`
- **Fecha:** `2026-10-26`
- **Descripción:** el panel se refresca más rápido y deja de usar emojis. En `apps/panel/src/App.tsx` los sondeos pasan a 2 s para `/v1/system`, `/v1/health`, `/v1/session` y `/v1/logs`, y a 5 s para `/v1/stats`; el snapshot del servidor (`server/system_monitor.py`) se cachea 3 s en vez de 10 s para no lanzar `systemctl`/`ss` en cada petición. Se quitan los emojis de los toasts (`Relay reiniciado (active)`), del KPI de energía (`AC`) y del menú de la bandeja (`Relay: OK`, `Relay: degradado`, `Relay: sin conexión`); el semáforo de la app pasa a sondear cada 5 s. Build web regenerado (`index-Dp5nL5mO.js`, `index-BcTWnyd2.css`). Verificado: `npm run build` del panel y de la app, `cargo check --workspace`, `cargo fmt -p atlas-panel -- --check` y búsqueda sin emojis en los builds. Desplegado con `./scripts/deploy-relay.sh --push`: `/panel/assets/index-Dp5nL5mO.js` 200 (232850 B), el asset anterior `index-DSAldxug.js` 404 y `/v1/health` OK. El bundle Tauri se reconstruyó con Apple Development (firma profunda válida), se instaló con `ditto` en `~/Applications/Hub Panel.app` y se relanzó: el binario embebe `index-Dp5nL5mO.js` y las etiquetas `Relay: OK` / `Relay: degradado` / `Relay: sin conexión` sin emojis, mantiene 4 conexiones HTTP reutilizadas contra `atlas-hub` y se cerró la instancia antigua que corría desde `target/release`.
- **Documentación relacionada:** `docs/panel-app-plan.md` (§ Rediseño de la UI del panel), `docs/relay.md` (§6), `docs/relay-api.md` (`/v1/system`), `AGENTS.md` (§ Semáforo).

### Hub Panel: rediseño de portada e iconos propios

- **Commit:** `feat(panel): rediseño de portada e iconos propios de Hub Panel`
- **Fecha:** `2026-10-26`
- **Descripción:** portada nueva en la UI compartida (`apps/panel`, sirve a `/panel/` y a la app): topbar fija con marca `HUB PANEL`, hora del snapshot y píldora de estado del relay (LED + servicios activos / en vuelo / cola), fila de 6 KPIs (servicios, peticiones, memoria, disco, uptime y energía) y tarjetas de Relay Atlas, Peticiones y Log a ancho completo; la antigua tarjeta `MacCard` desaparece porque sus datos pasan a los KPIs. Se añaden iconos propios de Hub Panel (rombo cian en el bundle, rombo verde/ámbar/rojo en la bandeja) en lugar de los copiados de la app principal. Build web regenerado (`index-DSAldxug.js`, `index-BcTWnyd2.css`) y bundle `Hub Panel.app` reconstruido con Apple Development y firma profunda válida. Verificado: `npm run build` de la app principal, `npm run build` del panel, `cargo check --workspace`, `cargo fmt -p atlas-panel -- --check` y `codesign --verify --deep --strict` OK. Desplegado en `atlas-hub` tras publicar `655b33a` con `./scripts/deploy-relay.sh --pull`: relay `active`, `/v1/health` OK y `/panel/` sirve los assets nuevos (`index-DSAldxug.js` 200, `index-BcTWnyd2.css` 200; el viejo `index-DaaFBFVS.js` devuelve 404). Nota: `dist/` está ignorado por `.gitignore`; los assets del panel se versionan con `git add -f`.
- **Documentación relacionada:** `docs/panel-app-plan.md` (§ Rediseño de la UI del panel), `docs/relay.md` (§6, §10), `AGENTS.md` (§ Builds separados).

### Fix: URL del relay por nombre en frontal y núcleo Tauri

- **Commit:** `fix(frontend): usar atlas-hub.local como URL por defecto del relay`
- **Fecha:** `2026-10-26`
- **Descripción:** el frontal web en dev (`src/App.tsx`) y el núcleo Tauri (`src-tauri/src/lib.rs`) dejan de usar la IP `192.168.100.26` como defecto y pasan a `http://atlas-hub.local:8765`, coherente con `ATLAS_RELAY_URL` y con la regla de no hardcodear IPs (la IP del servidor puede cambiar). Verificado con `npm run build` y `cargo check --workspace`.
- **Documentación relacionada:** `docs/relay-api.md` (base por defecto), `AGENTS.md` (§ URL del relay).

### Hub Panel: renombrado del bundle y ventana con Dock/Cmd+Tab

- **Commit:** `feat(panel-app): renombrar el bundle a Hub Panel y ventana con Dock/Cmd+Tab`
- **Fecha:** `2026-10-25`
- **Descripción:** la app pasa a llamarse **Hub Panel** en todo lo visible (`productName` → bundle `Hub Panel.app`, título de ventana, tooltip del icono, notificaciones y logs), manteniendo a propósito el identificador `com.atlas.panel` porque de él cuelgan `~/Library/Application Support/com.atlas.panel/` (clave SSH del reinicio y marcador de autoarranque) y el binario interno `Contents/MacOS/atlas-panel`. Se corrige además el comportamiento de ventana: con `ActivationPolicy::Accessory` fijo la app no salía en Cmd+Tab ni tenía menú superior; ahora `set_window_policy()` alterna `Regular` con la ventana visible (Dock, Cmd+Tab y menú) y `Accessory` al cerrarla (solo icono de barra), aplicado en `setup`, `show_panel()` y `CloseRequested`. Migración local: `launchctl bootout` del LaunchAgent antiguo, borrado de `~/Library/LaunchAgents/ATLAS Panel.plist`, borrado del marcador `autostart-initialized`, sustitución de `~/Applications/ATLAS Panel.app` por `Hub Panel.app` con `ditto` y primer arranque para que el plugin registre `Hub Panel.plist` (`RunAtLoad=true`, ruta nueva). Verificado: firma profunda válida, `CFBundleName = Hub Panel`, `CFBundleExecutable = atlas-panel`, app corriendo desde `~/Applications/Hub Panel.app`, `lsappinfo` con `type="Foreground"`, relay en verde, y confirmación del usuario de Dock + Cmd+Tab + menú superior con la ventana abierta y solo icono de barra al cerrarla. `cargo check -p atlas-panel` y `cargo fmt --check` OK.
- **Documentación relacionada:** `AGENTS.md` (§ App Hub Panel: ventana, Dock/Cmd+Tab y nombre), `docs/panel-app-plan.md` (§6, §7 "Renombrado a Hub Panel").

### Watchdog del relay: reinicio automático en atlas-hub (F5)

- **Commit:** `feat(health): watchdog systemd que reinicia el relay si no responde (F5)`
- **Fecha:** `2026-10-25`
- **Descripción:** red de seguridad en el servidor, independiente del Mac. `deploy/server/atlas-healthcheck.py` (solo stdlib) comprueba `/v1/health` en la IP LAN autodetectada (nunca loopback), con timeout 3 s, y exige HTTP 200 + `ok: true`; tras **2 fallos consecutivos** ejecuta `systemctl restart atlas-relay`. Guarda el contador en `/var/lib/atlas-health/failures` y lo reinicia tras cada intento (si el servicio no levanta, espera otra tanda en vez de martillear cada minuto); si el servicio está `activating`/`deactivating` no lo pisa. Modo seco (`ATLAS_HEALTH_DRY_RUN=1`) y overrides de URL/timeout/umbral/estado/unidad para poder probarlo sin tocar el servicio; exit 0 salvo error real al reiniciar. Unidades: `atlas-health.service` (oneshot, root, `StateDirectory`, `TimeoutStartSec=45`, `Nice`, `NoNewPrivileges`, `ProtectHome`, `PrivateTmp`, `ProtectKernelTunables`, `ProtectControlGroups`) y `atlas-health.timer` (90 s tras arranque, luego 60 s). Instalación única con `scripts/deploy-healthcheck.sh` (copia las unidades, `sudo` interactivo, `daemon-reload`, `enable --now` y primera pasada; aborta si el script no está ya en `/opt/atlas/repo`). Verificado: en local sano contra el relay real y en seco con `1/2` → `2/2` → `DRY RUN: reiniciaría`; en `atlas-hub` timer `enabled`/`active` con primera pasada `Result=success` y prueba de caída real: relay parado a las 23:24, reinicio automático a las 23:26:01 y `/v1/health` 200 a las 23:26:05, contador a 0. `NRestarts` sigue en 0 porque systemd no cuenta los reinicios manuales.
- **Documentación relacionada:** `docs/relay.md` (§9 watchdog y su operación), `docs/panel-app-plan.md` (§7 F5), `AGENTS.md` (watchdog y mantenimiento).

### ATLAS Panel: aviso macOS al caer el relay (F4)

- **Commit:** `feat(panel-app): aviso macOS al caer el relay (F4)`
- **Fecha:** `2026-10-25`
- **Descripción:** la app de barra de menús avisa cuando el relay deja de responder. Añade `tauri-plugin-notification` (2.7) registrado en Rust (`NotificationExt`); el bucle de salud, al detectar la transición a `RelayStatus::Down`, muestra una notificación `ATLAS · relay sin conexión` con el cuerpo `No se puede alcanzar <ATLAS_RELAY_URL>. Abre el panel para reiniciarlo.` Solo avisa al **entrar** en rojo (nunca en cada sondeo) y con un enfriamiento de 300 s para relays que oscilan; si al arrancar ya está caído avisa una vez. `on_action` del plugin: un clic (`tap`) abre la ventana del panel, sin reiniciar nada por su cuenta (la doble confirmación sigue en la UI). En dev el aviso está silenciado para no llenar el centro de notificaciones en cada recarga, con escape `ATLAS_PANEL_NOTIFY=1`. No requiere permisos nuevos en `capabilities/default.json` porque se emite desde Rust. Verificado: en dev con `ATLAS_PANEL_NOTIFY=1 ATLAS_RELAY_URL=http://127.0.0.1:9` el estado pasa a `relay sin conexión` y el aviso se emite sin error; en el bundle instalado lanzado con `open -n -a "$HOME/Applications/ATLAS Panel.app" --env ATLAS_RELAY_URL=http://127.0.0.1:9` aparece la notificación y el clic abre la ventana (confirmado por el usuario); vuelta a la normalidad con `/v1/health` 200. `cargo check --workspace`, `cargo test -p atlas-panel` y `cargo fmt --check` OK; bundle reconstruido con Apple Development (14,2 MiB), firma profunda válida e instalado en `~/Applications`.
- **Documentación relacionada:** `docs/panel-app-plan.md` (§7 F4, §8), `AGENTS.md` (§ App ATLAS Panel: notificaciones).

### ATLAS Panel: reinicio del relay por SSH con clave restringida (F3)

- **Commit:** `feat(panel-app): reinicio del relay por SSH con doble confirmación (F3)`
- **Fecha:** `2026-10-25`
- **Descripción:** la app de barra de menús puede reiniciar el relay sin depender de nadie delante del servidor. Clave `ed25519` exclusiva en `~/Library/Application Support/com.atlas.panel/keys/panel_ed25519` (`600`, fuera del repo), instalada en `~adria_admin/.ssh/authorized_keys` de `atlas-hub` con comando forzado `sudo systemctl restart atlas-relay && sudo systemctl is-active atlas-relay` y `no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding`; el `sudo` sin contraseña lo aporta `deploy/server/atlas-deploy.sudoers`. El comando IPC `restart_relay` no acepta parámetros: valida la clave, ejecuta `/usr/bin/ssh` con host fijo `atlas-hub`, `BatchMode`, `IdentitiesOnly`, `StrictHostKeyChecking=yes` y timeout de 25 s, y devuelve `{ ok, state, output }`. En la UI compartida (`apps/panel`) aparece "Reiniciar relay" solo dentro de Tauri, con `RestartCard` de dos pasos; el menú de la barra activa "Reiniciar relay…" (abre la ventana y emite `panel://restart-requested`) y el resultado se muestra como toast. Verificado: prueba directa con la clave (`rc=0`, `active`, relay de nuevo OK en `192.168.100.26:8765`), `cargo test -p atlas-panel -- --ignored` con `consulta_health_del_relay_real` y `reinicia_el_relay_real` en verde (1,42 s), bundle reconstruido con Apple Development, firma profunda válida e instalado en `~/Applications`, panel web regenerado con el nuevo botón, `cargo check --workspace`, `cargo test -p atlas-panel` y `cargo fmt --check` OK. Prueba de caída real con el usuario: `atlas-relay` parado en `atlas-hub` (`inactive`, `/v1/health` sin respuesta) → semáforo rojo y reinicio desde la UI con doble confirmación → `active`, toast `✅ Relay reiniciado (active)`, health OK y relay de nuevo escuchando en `192.168.100.26:8765`. Desplegado en `atlas-hub` con `./scripts/deploy-relay.sh --pull` (`/v1/health` OK): `/panel/` sirve `index-DaaFBFVS.js` y `index-CQ9_Qwgq.css` (200) y el asset antiguo devuelve 404. Nota: la comprobación de cambios sin commitear del script abortaba también con `--push` (solo `--pull` la evitaba); se desplegó con `--pull` porque los WIP de `src/` siguen abiertos, y el fix quedó en el commit `fix(deploy): --push ya no aborta por cambios sin commitear`.
- **Documentación relacionada:** `docs/panel-app-plan.md` (§4, §7 F3, §8), `AGENTS.md` (§ App ATLAS Panel: reinicio y clave), `docs/relay.md` (§8 operación de la clave del panel).

### ATLAS Panel: bandeja con semáforo, autoarranque y bundle firmado (F2)

- **Commit:** `feat(panel-app): bandeja con semáforo, autoarranque y bundle firmado (F2)`
- **Fecha:** `2026-10-25`
- **Descripción:** la app independiente `apps/panel-app` pasa a ser una app de barra de menús. Añade `TrayIconBuilder` con id `main`, iconos de semáforo embebidos (`icons/tray-ok|warn|down.png`), tooltip y menú (estado, "Abrir panel", "Reiniciar relay…" deshabilitado hasta F3, "Abrir al iniciar sesión" y "Salir" con ⌘Q). Un bucle async sondea `/v1/health` cada 10 s: verde si `ok:true`, ámbar si el relay responde mal (HTTP no-2xx o JSON sin `ok:true`), rojo si no hay red/timeout; solo actualiza icono/menú al cambiar de estado y emite `panel://relay-status`. Cerrar la ventana (Cmd+W) ahora la oculta en vez de terminar la app; se reabre con clic izquierdo en el icono o "Abrir panel". En macOS usa `ActivationPolicy::Accessory` (sin Dock). Autoarranque con `tauri-plugin-autostart` (LaunchAgent `~/Library/LaunchAgents/ATLAS Panel.plist`, activado en el primer arranque del bundle y desactivable desde el menú, con marcador para no reactivarse solo; deshabilitado en dev). Bundle `ATLAS Panel.app` 0.1.0 firmado con Apple Development (`codesign --verify --deep --strict` válido) e instalado en `~/Applications`. Verificado en dev con relay real (`relay OK` + UI por IPC) y con `ATLAS_RELAY_URL=http://127.0.0.1:9` (`relay sin conexión`, app viva); arranque del bundle instalado, LaunchAgent creado con la ruta correcta y probado con `launchctl bootstrap` (launchd arrancó la app: `state = running`, `runs = 1`), y login real confirmado por el usuario; `cargo check --workspace`, `cargo test -p atlas-panel` y `cargo fmt --check` OK. Reglas de uso/refresco/lanzamiento/bundle/autoarranque añadidas a `AGENTS.md`.
- **Documentación relacionada:** `docs/panel-app-plan.md` (§7 F2 y F2 extra), `AGENTS.md` (§ App ATLAS Panel).

### ATLAS Panel: scaffold de la app macOS independiente (F1)

- **Commit:** `feat(panel-app): scaffold Tauri v2 con UI compartida y comandos HTTP`
- **Fecha:** `2026-10-25`
- **Descripción:** nueva app de escritorio independiente de la app principal en `apps/panel-app` (Tauri v2, identifier `com.atlas.panel`, ventana y bundle propios, versión `0.1.0`). Reutiliza el frontend de `apps/panel` con dos builds: el del relay sigue con base `/panel/` en `apps/panel/dist` y el de la app usa base relativa (`PANEL_BASE=./`) y sale a `apps/panel-app/dist` (`PANEL_OUT_DIR`). `apps/panel/src/api.ts` detecta Tauri (`isTauri()`) y envía las peticiones por los comandos IPC `api_get`/`api_post`, implementados en Rust con `reqwest` (timeout 8 s); en navegador sigue usando `fetch` con CORS. El backend valida que la ruta empiece por `/v1/` y rechaza URLs absolutas, `..`, fragmentos y espacios, de modo que el webview no puede usar el comando como proxy. Añadido `apps/panel-app/src-tauri` al workspace de Cargo. Verificado: `cargo check --workspace`, `cargo test -p atlas-panel` (validación de rutas + `/v1/health` real contra `atlas-hub`), `npm run build` de la app principal y arranque real con `tauri dev` (la UI sondea `/v1/system`, `/v1/health`, `/v1/session`, `/v1/stats` y `/v1/logs` por IPC). `npm run lint` mantiene 4 errores previos en `apps/panel` (refs durante render y fast-refresh), ajenos a este cambio.
- **Documentación relacionada:** `docs/panel-app-plan.md` (§7 F1), `docs/index.md`.

### Fix: el relay ya no puede quedar escuchando en loopback si arranca sin red

- **Commit:** `fix(relay): reintentar autodetección de IP LAN al arrancar sin red`
- **Fecha:** `2026-10-25`
- **Descripción:** tras el reinicio de `atlas-hub` a las 11:07 el relay arrancó 5 s después, antes de que el Wi-Fi/DHCP tuviera IP; `lan_ip()` cayó al fallback `127.0.0.1` y el relay quedó activo pero inalcanzable desde la LAN (panel y frontal sin interactuar). Como el socket de loopback se abre sin error, `Restart=on-failure` no se disparaba. Ahora, cuando `RELAY_HOST` está vacío y la autodetección devuelve loopback, el relay reintenta cada 2 s hasta 60 s y, si sigue sin IP LAN, sale con error de config (exit 2) para que systemd lo reinicie. Verificado en `atlas-hub`: tras reiniciar quedó escuchando en `192.168.100.26:8765`, `/v1/health` OK y `/panel/` 200.
- **Documentación relacionada:** `docs/relay.md` (§10 troubleshooting, §11 notas de instalación).

### Panel ATLAS recortado a atlas-hub: fuera docker/nube/unidades fantasma y unidades de usuario

- **Commit:** `feat(panel): recortar panel a atlas-hub (sin docker/nube/unidades fantasma)`
- **Fecha:** `2026-10-24`
- **Descripción:** auditoría de servicios en `atlas-hub` contra la docu: solo están instalados y operativos `ssh.service`, `avahi-daemon.service` y `atlas-relay.service` (unidad de sistema idéntica al repo); `ufw` está instalado y `enabled` pero **INACTIVA** (pendiente de activar con sudo interactivo). NO existen en atlas-hub: docker (sin binario/socket), tailscale, cups, macfan, lid-monitor, smartmontools, NetworkManager (la red la gestiona ifupdown), el compose de Nextcloud (`/home/adria/nextcloud/…`) ni las piezas de host del panel antiguo (polkit `50-atlas-panel.rules`, sudoers `atlas-panel`, ayudante `panel-docker.sh`). Corte aplicado: en `server/system_monitor.py` desaparecen docker/nube/tailscale del snapshot (`GET /v1/system` sin claves `docker`/`cloud`), el alcance de usuario (sin linger) y el ayudante sudo; `PANEL_ACTION_UNITS` queda **VACÍA** por defecto (solo se reactiva por `.env` al integrar unidades + reglas polkit); unidades por defecto ahora `ssh, avahi-daemon, ufw, atlas-relay`; `KNOWN_PORTS` reduce a 22/8765. En `server/atlas_relay.py` el flujo asks/resolve solo acepta `kind: "service"` y `cancel_all` (fuera compose). Frontend `apps/panel/`: eliminadas tarjetas Contenedores/Docker y Nube·Nextcloud (y sus tipos), rótulo `homelab`→`atlas-hub`; build nuevo desplegado vía Git (assets hash renombrados). Docu actualizada: `docs/relay.md` (§2 estado real ufw, §6 reescrito, §7 nota de nube, §9/§10/§11) y `docs/relay-api.md` (schema `/v1/system`, kinds de asks y variables `PANEL_*`). Nota: el unit systemd del repo cambia solo en un comentario (el instalado sigue funcional hasta un próximo paso con sudo interactivo).
- **Documentación relacionada:** `docs/relay.md` (§2, §6, §7), `docs/relay-api.md`.

### Panel ATLAS v1: observador del sistema, acciones con doble confirmación y /panel estático

- **Commit:** `feat(relay): observador /v1/system, acciones con doble confirmación y /panel estático` (`docs:` previo: `docs: manual de operaciones del relay y API del panel`)
- **Fecha:** `2026-10-23`
- **Descripción:** nuevo módulo `server/system_monitor.py` (unidades systemd sistema+usuario, puertos `ss`, docker vía ayudante sudo restringido `/usr/local/lib/atlas/panel-docker.sh`, nube Nextcloud + `tailscale serve`, recursos/batería/temperatura) con caché de 10 s. Endpoints: `GET /v1/system`, `POST /v1/system/asks` → `POST /v1/system/resolve` (doble confirmación, op-id TTL 30 s, ya sea acción de unidad, compose app/db o cancel_all) y `POST /v1/system/cancel_all`; `Brain.cancel_all()` + trackeo `inflight`; `/v1/health` ampliado (`bot.username` de `getMe`, `queue`, `inflight`); CORS multi-origen (eco del Origin solicitante), estáticos del panel en `/panel/*`. Frontend `apps/panel/` (React 19 + Vite, base `/panel/`, sin router) con tarjetas: Servicios systemd (start/stop/restart), Puertos, Contenedores, Nube Nextcloud, Este Mac (RAM/disco/temp/batería), Relay·Telegram (bot, tokens por modelo, cola), Peticiones (mensajes con origen TG/local + cancelar) y Log del relay; tarjeta ámbar de confirmación clonada del `ConfirmationPanel` del frontal. Permisos en homelab: polkit `50-atlas-panel.rules` (whitelist docker/cups/avahi/macfan/lid-monitor/smartmontools; sshd/tailscaled/NetworkManager excluidos a propósito) y sudoers `/etc/sudoers.d/atlas-panel` para el ayudante. Panel verificado en `http://192.168.100.26:8765/panel/` con acción real ejecutada tras confirmación.
- **Documentación relacionada:** `docs/relay.md` (manual de operaciones para el Mac de desarrollo), `docs/relay-api.md`, `docs/index.md`.

### Sync del relay: procesar historial y confirmaciones en primera sync + apuntar a homelab

- **Commit:** `feat(frontend): procesar historial y confirmaciones en primera sync del relay y apuntar a homelab` (`770c1dd`)
- **Fecha:** `2026-10-23`
- **Descripción:** la primera sincronización de `pollDevRelay()` ahora procesa mensajes del historial (sin animar letras), confirmaciones pendientes y confirmaciones resueltas, en lugar de solo marcar todo como visto. La URL por defecto del relay en Tauri cambia de `127.0.0.1` a `192.168.100.26` (homelab). Consolidada la documentación de reglas de construcción y pruebas en `AGENTS.md`; eliminados `docs/construction-rules.md` y `docs/testing.md`. Desplegado en homelab con `./scripts/deploy-relay.sh --push`; relay verificado sano en `http://192.168.100.26:8765/v1/health` (`{"ok": true, "telegram": "enabled", "llm": "groq", "calendar": "standby", "events": 0}`).
- **Documentación relacionada:** `docs/relay-api.md`, `AGENTS.md`.

### Fix: mensajes de Telegram no aparecían al abrir el frontend (modo dev)

- **Commit:** `Fix dev: procesar mensajes del historial en la primera sincronización con el relay`
- **Fecha:** `2026-10-18`
- **Descripción:** en modo dev, cuando se enviaban mensajes por Telegram
  sin el frontend activo y luego se abría la app, los mensajes de usuario no
  aparecían — solo las respuestas del LLM. Causa: la primera sincronización
  de `pollDevRelay()` marcaba todos los eventos como vistos (incluidos los
  mensajes del historial) sin procesarlos. Fix: la primera sync ahora también
  procesa mensajes (sin animar letras), confirmaciones y estados. Verificado:
  `npm run build`, `npm run lint`, `cargo check --workspace`.
- **Documentación relacionada:** `docs/relay-api.md`.

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

