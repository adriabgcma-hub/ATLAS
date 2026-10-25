# ATLAS Panel — app macOS independiente (plan v1)

Estado: **F1–F4 implementadas** (barra de menús, autoarranque, reinicio del
relay por SSH con clave restringida y aviso macOS al caer el relay); bundle
firmado y probado (2026-10-25; F4 pendiente de commit). Sustituye al panel web
servido por el relay como *herramienta de control*, sin retirar `/panel/` (sigue
disponible para otros clientes de la LAN).

Objetivo: una app de escritorio **independiente de la app principal de ATLAS**
que permita ver el relay y **recuperarlo** (reinicio) aunque esté colgado, y
viceversa: si esta app falla, la principal sigue funcionando.

## 1. Decisiones fijadas

| Tema | Decisión |
|---|---|
| Tecnología | **Tauri v2** (Rust + React ya existente) |
| Formato | **App de barra de menús** (tray) + ventana al pulsar |
| UI | **Reutiliza `apps/panel`** (un solo frontend) |
| Canal de reinicio | **SSH con clave restringida** (comando forzado) |
| Red de seguridad | **Watchdog systemd en `atlas-hub`** (independiente del Mac) |
| Distribución | Local, firma **Apple Development**, sin App Store |
| Nombre / bundle | `ATLAS Panel` · `com.atlas.panel` |
| Avisos | **Notificación macOS** al caer el relay |
| Autoarranque | Al login, **off por defecto** |

## 2. Independencia de la app principal

- Bundle, proceso, icono, versión e identifier propios: no comparte runtime ni
  ciclo de actualización con la app principal.
- **Empaqueta su propio build** del panel dentro del bundle → no depende de que
  el relay sirva `/panel/`.
- Si el relay no responde: la app muestra estado "sin conexión" y ofrece el
  botón de reinicio (el caso de uso principal).
- La app principal no depende de esta; ambas solo comparten el relay (API HTTP).

## 3. Estructura de carpetas

```
apps/panel/          UI compartida React + Vite (se mantiene)
apps/panel-app/      shell Tauri v2 (nueva)
  src-tauri/
    tauri.conf.json  identifier com.atlas.panel · tray · ventana
    Cargo.toml
    src/main.rs      comandos HTTP, reinicio SSH, tray
    icons/
  package.json       script que builda la UI con PANEL_BASE=./ → dist
```

- `apps/panel/vite.config.ts`: base parametrizable por entorno
  (`PANEL_BASE`; defecto `/panel/` para el relay, `./` para la app).
- HTTP: **comandos Rust** (`health`, `snapshot`, `logs`, `session`, `stats`)
  con `reqwest` → **sin CORS** y sin tocar `RELAY_CORS_ORIGIN` del servidor.
- `restart_relay`: ejecuta un **único comando fijo** por SSH; el backend no
  acepta comandos arbitrarios desde la UI.
- Doble confirmación en la UI para reiniciar (regla ATLAS: sin permiso
  implícito).

## 4. Clave SSH restringida (corazón del reinicio)

- Clave `ed25519` **exclusiva de la app**, sin passphrase, permisos `600`,
  fuera del repo:
  `~/Library/Application Support/com.atlas.panel/keys/panel_ed25519`
- Entrada en `~adria_admin/.ssh/authorized_keys` de `atlas-hub` con comando
  forzado y sin reenvíos:

```
command="sudo systemctl restart atlas-relay && sudo systemctl is-active atlas-relay",no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding ssh-ed25519 AAAA... atlas-panel
```

- Esa clave **solo** puede reiniciar `atlas-relay` (y consultar su estado);
  el sudoers mínimo `atlas-deploy` ya permite esas dos operaciones sin
  contraseña.
- Rotación: eliminar la línea y regenerar la clave.

## 5. Watchdog en el servidor (independiente del Mac)

- `deploy/server/atlas-healthcheck.py` (solo stdlib) +
  `atlas-health.service` (oneshot, root) + `atlas-health.timer` (60 s).
- Comprueba `http://<IP LAN>:8765/v1/health`; tras **2 fallos seguidos**
  reinicia `atlas-relay`.
- Funciona con el Mac apagado: cubre el caso "no hay nadie mirando".

## 6. Comportamiento de la app

- **Tray**: icono con semáforo (ok / warn / down) y menú con estado, "Abrir
  panel", "Reiniciar relay…", "Abrir al iniciar sesión" y "Salir".
- **Ventana**: mismas tarjetas del panel (servicios, puertos, Mac, relay,
  peticiones, logs) + botón "Reiniciar relay" (solo en la app) con doble
  confirmación.
- **Notificaciones**: una por caída (sin spam), con enlace a la ventana.
- **Autoarranque**: toggle con `tauri-plugin-autostart`, apagado por defecto.

## 7. Fases

| Fase | Contenido |
|---|---|
| F0 | Este plan + docs |
| F1 | Scaffold Tauri, build de UI con `PANEL_BASE=./`, comandos HTTP — **hecha** |
| F2 | Tray + polling de salud + ventana — **hecha** |
| F3 | Clave restringida + reinicio con doble confirmación (comandos de instalación para `atlas-hub`) — **hecha** |
| F4 | Notificaciones + autoarranque — **hecha** (aviso macOS al caer el relay y autoarranque al login) |
| F5 | Watchdog systemd + despliegue |
| F6 | `tauri build` (Apple Development), firma profunda y prueba de caída real — **bundle firmado**; prueba de caída pendiente |

### F1: qué hay ya

- `apps/panel-app/` con shell Tauri v2 (`com.atlas.panel`, ventana propia) y
  `src-tauri/src/main.rs` con los comandos `api_get` / `api_post`
  (`reqwest`, timeout 8 s). El backend **solo acepta rutas `/v1/...`**; el
  webview no puede elegir host ni convertir el comando en un proxy.
- `apps/panel/src/api.ts`: si corre dentro de Tauri usa `invoke`; si corre en
  el navegador (panel del relay) mantiene `fetch` + CORS. Un solo frontend.
- `apps/panel/vite.config.ts`: `PANEL_BASE` y `PANEL_OUT_DIR` parametrizables
  (defecto `/panel/` y `dist` para el relay).
- Builds:
  - relay: `cd apps/panel && npm run build` → `apps/panel/dist` (base `/panel/`)
  - app: `cd apps/panel-app && npm run build:ui` → `apps/panel-app/dist`
    (base `./`, empaquetado en el bundle)
- Dev de la app: `cd apps/panel-app && npm run dev` (Vite del panel en 5174 +
  ventana Tauri; los `invoke` se registran en el log solo en debug).
- Iconos: de momento copiados de la app principal; el icono propio llega en F6.
- Verificado: `cargo check --workspace`, `cargo test -p atlas-panel`
  (2 tests de validación de rutas + prueba real de `/v1/health` con
  `--include-ignored`), `npm run build` de la app principal y arranque real de
  `tauri dev` con la UI sondeando el relay por IPC (`/v1/system`, `/v1/health`,
  `/v1/session`, `/v1/stats`, `/v1/logs`).

### F2: qué hay ya

- **Barra de menús:** `TrayIconBuilder` con id `main`, icono de semáforo
  embebido en el binario (`icons/tray-ok|warn|down.png`, 44×44) y tooltip
  `ATLAS Panel · relay <estado>`.
- **Sondeo de salud:** cada 10 s a `/v1/health`. `ok:true` → verde; respuesta
  HTTP no-2xx o JSON sin `ok:true` → ámbar (degradado); error de red/timeout →
  rojo (sin conexión). Solo se actualiza el icono/menú cuando cambia el
  estado, y se emite el evento `panel://relay-status` para fases siguientes.
- **Menú:** estado actual (deshabilitado), separador, "Abrir panel",
  "Reiniciar relay…" (deshabilitado hasta F3) y "Salir" (⌘Q).
- **Ventana:** cerrarla (Cmd+W) la oculta y la app sigue en la barra; clic
  izquierdo en el icono o "Abrir panel" la vuelve a mostrar. `Cmd+Q` o
  "Salir" terminan la app.
- **macOS:** política de activación `Accessory` (sin icono en el Dock).
- Verificado en dev: arranque real con relay accesible → `relay OK` y la UI
  sondeando por IPC; arranque con `ATLAS_RELAY_URL=http://127.0.0.1:9` →
  `relay sin conexión` sin que la app muera; `cargo test -p atlas-panel`,
  `cargo fmt --check` y `cargo check -p atlas-panel`.
- **Reglas de uso/refresco:** documentadas en `AGENTS.md` § "App ATLAS Panel".

### F2 extra: autoarranque, instalación y bundle firmado

- **Autoarranque:** `tauri-plugin-autostart` con `MacosLauncher::LaunchAgent`.
  En el primer arranque del bundle (no en dev) se activa y se escribe el
  marcador `autostart-initialized` en
  `~/Library/Application Support/com.atlas.panel/`. Genera
  `~/Library/LaunchAgents/ATLAS Panel.plist` con `RunAtLoad=true` y la ruta del
  `.app`. El menú incluye "Abrir al iniciar sesión" (checkbox); si el usuario lo
  desactiva, el marcador evita que se reactive solo.
- **Instalación local:** el bundle se copia con `ditto` a
  `~/Applications/ATLAS Panel.app` y se lanza desde ahí, para que el
  LaunchAgent no apunte a `target/release` (se puede limpiar).
- **Build firmado:** `APPLE_SIGNING_IDENTITY="Apple Development: …" npm run
  build` (identidad del llavero, `security find-identity -v -p codesigning`).
  Resultado: `target/release/bundle/macos/ATLAS Panel.app` (13,5 MiB),
  `codesign --verify --deep --strict` válido y `com.atlas.panel` / `0.1.0` en
  `Info.plist`. La notarización se omite a propósito (uso local).
- **Verificado:** arranque del `.app` instalado, proceso
  `~/Applications/ATLAS Panel.app/Contents/MacOS/atlas-panel` activo y
  LaunchAgent creado con la ruta correcta; además se cargó con
  `launchctl bootstrap gui/$(id -u) "~/Library/LaunchAgents/ATLAS Panel.plist"`
  y launchd arrancó la app (`state = running`, `runs = 1`). **Login real
  confirmado por el usuario (2026-10-25).** La notificación de caída se
  implementó en F4 (ver más abajo).

### F3: qué hay ya

- **Clave exclusiva del panel:** `ed25519` sin passphrase en
  `~/Library/Application Support/com.atlas.panel/keys/panel_ed25519` (modo
  `600`, directorio `700`; nunca en el repo). Huella
  `SHA256:wExdWnMDtEF/pJCP8+nTbbpjMjFkHgkQ+l/qTZKdcr0`.
- **Instalación en `atlas-hub`** (requiere permiso del usuario; modifica
  `~adria_admin/.ssh/authorized_keys`, quedó con 2 líneas, 1 `atlas-panel`):

```
command="sudo systemctl restart atlas-relay && sudo systemctl is-active atlas-relay",no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIFxB0/h/yGW5mREh35tI4JWCQFlukIjP2dp5CuvLXx0 atlas-panel
```

- **Rust (`restart_relay`)**: comando IPC sin parámetros. Valida que la clave
  exista (si falta, error explicativo), ejecuta `/usr/bin/ssh` con
  `-i <clave>`, `IdentitiesOnly=yes`, `BatchMode=yes`, `ConnectTimeout=8`,
  `StrictHostKeyChecking=yes`, host fijo `atlas-hub`, argumento inofensivo
  `panel-restart` y timeout global de 25 s. Devuelve
  `{ ok, state, output }`; `ok` es `state == "active"`.
- **UI (React)**: botón "Reiniciar relay" arriba a la derecha, visible solo
  dentro de Tauri (`IN_TAURI`), y `RestartCard` con dos pasos (Continuar → Sí,
  reiniciar). El menú de la barra activa "Reiniciar relay…", abre la ventana y
  emite `panel://restart-requested`. Al terminar muestra toast
  `✅ Relay reiniciado (active)` o el error. Nada de comandos arbitrarios: la UI
  solo invoca `restart_relay`.
- **Privilegios**: el `sudo` sin contraseña lo aporta
  `deploy/server/atlas-deploy.sudoers` (restart/is-active/status de
  `atlas-relay`); el comando forzado impide shell y reenvíos.
- **Verificado (2026-10-25):** prueba directa con la clave → `rc=0` en 1 s,
  salida `active`; `/v1/health` OK después y relay escuchando en
  `192.168.100.26:8765`; `cargo test -p atlas-panel -- --ignored` con
  `consulta_health_del_relay_real` y `reinicia_el_relay_real` en verde
  (1,42 s); bundle reconstruido, firmado, instalado en `~/Applications` y
  arrancado. El build web del panel (`apps/panel/dist`) se regeneró con el
  nuevo botón.
- **Prueba de caída real (confirmada por el usuario):** se paró
  `atlas-relay` en `atlas-hub` (`systemctl stop`, `inactive` y
  `/v1/health` sin respuesta); la app pasó a semáforo rojo y el reinicio desde
  la UI (doble confirmación) devolvió el servicio a `active`, con toast
  `✅ Relay reiniciado (active)`, `/v1/health` OK y el relay escuchando otra vez
  en `192.168.100.26:8765`. El caso "relay colgado" se cubrirá con el watchdog
  de F5.
- **Despliegue:** `./scripts/deploy-relay.sh --pull` (el check de cambios sin
  commitear abortaba también con `--push`; corregido en
  `fix(deploy): --push ya no aborta por cambios sin commitear`).
  Verificado en `atlas-hub`: `/v1/health` OK, `/panel/` sirve
  `index-DaaFBFVS.js` (200, 231 604 B) y `index-CQ9_Qwgq.css` (200), y el asset
  antiguo devuelve 404.

### F4: qué hay ya

- **Plugin:** `tauri-plugin-notification` (2.x) registrado en el builder;
  `tauri_plugin_notification::NotificationExt` en Rust. No hace falta permiso
  en `capabilities/default.json` porque el aviso se emite desde Rust, no desde
  la webview (`core:default` sigue siendo el único permiso).
- **Cuándo avisa:** solo al **entrar** en `RelayStatus::Down` (transición
  verde/ámbar → rojo), nunca en cada sondeo. Además hay un enfriamiento de
  300 s (`NOTIFY_COOLDOWN`) por si el relay oscila. Si al arrancar ya está
  caído, avisa una vez.
- **Contenido:** título `ATLAS · relay sin conexión`, cuerpo
  `No se puede alcanzar <ATLAS_RELAY_URL>. Abre el panel para reiniciarlo.`
  Sin rutas privadas ni secretos.
- **Clic:** `on_action` del plugin; `action_id == "tap"` → `show_panel`
  (la confirmación del reinicio sigue en la UI, no se reinicia solo).
- **En dev:** silenciado para no llenar el centro de notificaciones con cada
  recarga de `tauri dev`; `ATLAS_PANEL_NOTIFY=1` fuerza el aviso para poder
  probarlo sin montar el bundle.
- **Verificado (2026-10-25):**
  - En dev con `ATLAS_PANEL_NOTIFY=1 ATLAS_RELAY_URL=http://127.0.0.1:9`:
    estado `relay sin conexión` y aviso emitido sin error.
  - En el bundle instalado, lanzado con
    `open -n -a "$HOME/Applications/ATLAS Panel.app" --env
    ATLAS_RELAY_URL=http://127.0.0.1:9`: aparece la notificación y **al
    pulsarla se abre la ventana del panel** (confirmado por el usuario).
  - Vuelta a la normalidad sin la variable: proceso con el relay real y
    `/v1/health` 200.
- **Autoarranque:** ya cubierto en "F2 extra" (sigue activo; al login abre la
  ventana además del icono, tal como pidió el usuario).

## 8. Pruebas

- Dev: `cd apps/panel-app && npm run dev`; verificación visual con el usuario.
- Producción: `npm run build` (Tauri) y prueba de la app generada.
- **Reinicio (F3):** probar el botón y el menú de la barra (doble confirmación)
  contra el relay real; solo con permiso, porque corta el servicio unos
  segundos. Comprobar el toast `✅ Relay reiniciado (active)` y que el semáforo
  vuelve a verde. Equivalente por consola:
  `cargo test -p atlas-panel -- --ignored` (incluye `reinicia_el_relay_real`).
- **Notificaciones (F4):** lanzar el bundle con
  `open -n -a "$HOME/Applications/ATLAS Panel.app" --env
  ATLAS_RELAY_URL=http://127.0.0.1:9` (o `ATLAS_PANEL_NOTIFY=1` en dev):
  comprobar que el aviso aparece **una vez**, que el clic abre la ventana y que
  no se repite mientras el relay siga caído.
- **Simulación de fallo real**: `sudo systemctl stop atlas-relay` en `atlas-hub`
  (pide contraseña) → la app debe mostrarlo en rojo y recuperarlo desde la UI.
- Independencia: cerrar/eliminar una app y comprobar que la otra sigue.
- Verificar que no se exponen secretos ni rutas privadas en logs.

## 9. Riesgos y mitigaciones

- App con capacidad de reiniciar un servicio remoto → tratarla como
  privilegiada: clave restringida, comando forzado, confirmación explícita.
- Doble bundle y doble flujo de actualización → aceptado por diseño
  (independencia); versionado separado.
- UI duplicada → se evita reutilizando `apps/panel` con un build distinto.
