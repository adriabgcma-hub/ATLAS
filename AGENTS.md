# Instrucciones de trabajo

- Lee en  `docs/index.md` y lee las documentaciones necesarias

## Servidores de desarrollo (mantener levantados mientras se trabaja)

- Frontend: `npm run dev` → http://localhost:5173 (recarga sola con HMR).
- Relay Python: **corre en el servidor remoto** `atlas-hub` (`atlas-hub.local`,
  Debian 13; la IP puede cambiar: usa SIEMPRE el nombre, no la IP), no en
  local. Dirección: `http://atlas-hub.local:8765` (núcleo lo drena vía
  `ATLAS_RELAY_URL`; el frontal web en dev lo sondea directo con CORS solo
  para ese origen). El `.env` se carga automáticamente al iniciar.
- En `atlas-hub` el relay es el servicio systemd **de sistema**
  `atlas-relay.service` (`/etc/systemd/system/`, fuente versionada en
  `deploy/server/atlas-relay.service`): usuario de sistema `atlas` (home
  `/opt/atlas`, sin login ni sudo), repo en `/opt/atlas/repo`. Logs:
  `journalctl -u atlas-relay -f` (en `atlas-hub`).
- **Sincronizar y desplegar el relay** (desde la raíz del repo del Mac):
  `./scripts/deploy-relay.sh` → sube cambios commiteados, hace pull en
  `/opt/atlas/repo` como `atlas` (`sudo -u atlas`), reinicia el servicio
  (`sudo systemctl restart`) y comprueba `/v1/health`. El relay **no** recarga
  solo: cada despliegue lo reinicia el script.
  **Importante:** el script NO commitea automáticamente. Commitea manualmente
  antes de desplegar. Si hay cambios sin commitear, el script aborta (salvo
  `--pull`, que solo baja el remoto, o `--push`, que sube lo ya commiteado sin
  exigir árbol limpio).
- **Watchdog del relay** (`atlas-health.timer` en `atlas-hub`): instalación
  única con `./scripts/deploy-healthcheck.sh` (necesita la contraseña de
  `adria_admin`; el script del chequeo debe estar ya en `/opt/atlas/repo`, o
  sea: despliega el relay antes). Comprueba `/v1/health` en la IP LAN cada
  60 s y reinicia `atlas-relay` tras 2 fallos seguidos. Para parar el relay a
  mano, antes `sudo systemctl stop atlas-health.timer` y después `start`; si
  no, el watchdog lo levanta en 1-2 min. Detalle en `docs/relay.md` §9.
- El relay solo escucha en la IP LAN de `atlas-hub` (8765, autodetectada, no
  en `127.0.0.1`); el firewall `ufw` de `atlas-hub` debe dejar pasar
  `8765/tcp` SOLO desde `192.168.100.0/24`.
- Acceso SSH al servidor: alias `atlas-hub` del Mac (usuario administrador
  `adria_admin`, solo clave, `~/.ssh/id_ed25519`). El usuario del relay
  (`atlas`) no tiene login: el deploy usa el sudo sin contraseña mínimo de
  `deploy/server/atlas-deploy.sudoers` (pull como `atlas` + restart/is-active/
  status del servicio; ningún otro comando).
- Nunca commitees `.env` (ignorado por Git): lleva tokens y API keys reales.
  Para pasarlo a `atlas-hub` (queda en `/opt/atlas/repo/.env`, dueño `atlas`,
  modo 600): `scp .env adria_admin@atlas-hub:/tmp/atlas-env && ssh -t
  adria_admin@atlas-hub 'sudo install -m 600 -o atlas -g atlas /tmp/atlas-env
  /opt/atlas/repo/.env && rm -f /tmp/atlas-env'`.
- Tauri no lee `.env` solo: para `tauri dev`/`build` con claves,
  `set -a; . ./.env; set +a` antes (las pruebas Rust igual).
- iCloud CalDAV: prohibido martillear. Cada prueba contra iCloud (una sola
  petición de lectura o más) requiere permiso explícito del usuario, con pausa
  entre intentos. Apple limita (throttle 403) ante ráfagas.

## App Hub Panel (`apps/panel-app`)

App macOS **independiente** de la principal (barra de menús, bundle
`com.atlas.panel`): sirve para ver el relay y reiniciarlo aunque esté colgado.
Si una app falla, la otra sigue funcionando.

- **Una sola UI:** la fuente es `apps/panel`. No dupliques componentes ni
  estilos en `apps/panel-app`; todo cambio visual debe funcionar en los dos
  builds (web `/panel/` y app).
- **Lanzar en desarrollo:** `cd apps/panel-app && npm run dev` → levanta el
  Vite del panel en **5174** y abre la ventana Tauri. No ejecutes a la vez
  `cd apps/panel && npm run dev` (mismo puerto 5174); el frontal principal usa
  5173 y sí puede convivir.
- **Refresco:** los cambios en React (`apps/panel/src`) recargan solos (HMR).
  Los cambios en Rust (`apps/panel-app/src-tauri`) hacen que `tauri dev`
  recompile y reinicie la app. Si cambias `vite.config.ts`, `package.json` o
  dependencias, para y vuelve a lanzar `npm run dev`.
- **Builds separados:** la app usa `npm run build:ui` (base `./` y salida en
  `apps/panel-app/dist`, **ignorado por Git**; no lo commitees). El panel web
  del relay usa `cd apps/panel && npm run build` (base `/panel/`, salida
  `apps/panel/dist`, **sí** se commitea y se despliega con el relay).
- **Ventana:** cerrarla (Cmd+W) solo la oculta; la app sigue en la barra de
  menús. Se reabre con clic izquierdo en el icono o con "Abrir panel". Para
  salir de verdad: menú "Salir" o Cmd+Q.
- **Dock, Cmd+Tab y menú superior:** dependen de la política de activación.
  Con la **ventana visible** la app usa `ActivationPolicy::Regular` (sale en
  Dock, Cmd+Tab y menú superior, como una app normal); al cerrarla pasa a
  `Accessory` y queda solo el icono de la barra. Ese cambio lo hacen
  `set_window_policy()` en `show_panel()` y en `CloseRequested`: no lo
  sustituyas por un `Accessory` fijo o se pierden Cmd+Tab y el menú.
- **Nombre:** el bundle es **Hub Panel** (`Hub Panel.app`,
  `~/Applications/Hub Panel.app`, LaunchAgent `Hub Panel.plist`), pero el
  identificador sigue siendo **`com.atlas.panel`** a propósito: de él cuelgan
  `~/Library/Application Support/com.atlas.panel/` (clave SSH del reinicio,
  marcador de autoarranque) y el binario interno (`Contents/MacOS/atlas-panel`,
  nombre del crate). No cambies el identificador sin mover la clave y el
  marcador, o el reinicio deja de funcionar.
- **Semáforo:** sondea `/v1/health` cada 10 s (verde OK, ámbar degradado, rojo
  sin conexión). La app **no** reinicia sola: el reinicio es siempre manual y
  confirmado; en el servidor sí hay watchdog automático (`atlas-health.timer`,
  ver la sección del relay).
- **Reinicio del relay (F3):** solo mediante SSH con clave restringida y doble
  confirmación. Dos entradas: botón "Reiniciar relay" (arriba a la derecha,
  solo dentro de la app) y menú de la barra "Reiniciar relay…" (abre la ventana
  y emite `panel://restart-requested`). La confirmación (Continuar → Sí,
  reiniciar) vive en React; el comando IPC `restart_relay` no acepta parámetros
  y ejecuta siempre `ssh -i <clave> atlas-hub panel-restart` (host fijo,
  timeout 25 s). El comando remoto real está en `authorized_keys`:
  `command="sudo systemctl restart atlas-relay && sudo systemctl is-active
  atlas-relay",no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding`
  → la clave no puede abrir shell ni ejecutar otra cosa. El `sudo` sin
  contraseña lo aporta `deploy/server/atlas-deploy.sudoers` (restart/is-active/
  status de `atlas-relay`).
- **Clave del panel:** `~/Library/Application Support/com.atlas.panel/keys/
  panel_ed25519` (ed25519, sin passphrase, `600`, fuera del repo; huella
  `SHA256:wExdWnMDtEF/pJCP8+nTbbpjMjFkHgkQ+l/qTZKdcr0`, comentario
  `atlas-panel`). Instalarla o rotarla en `atlas-hub` requiere **permiso
  explícito del usuario** (modifica `~adria_admin/.ssh/authorized_keys`). Para
  rotar: generar clave nueva, añadir su línea con el mismo comando forzado,
  probar el reinicio y solo entonces borrar la anterior.
- **Notificaciones (F4):** `tauri-plugin-notification` avisa **una vez por
  caída** (solo al pasar a rojo, con enfriamiento de 5 min) y el clic abre la
  ventana; no reinicia nada solo. En dev está silenciado: para probarlo, lanza
  `ATLAS_PANEL_NOTIFY=1` (dev) o el bundle con
  `open -n -a "$HOME/Applications/Hub Panel.app" --env
  ATLAS_RELAY_URL=http://127.0.0.1:9`. Como el aviso se emite desde Rust, no
  necesita permisos nuevos en `capabilities/default.json`.
- **URL del relay:** `ATLAS_RELAY_URL` (por defecto
  `http://atlas-hub.local:8765`). No hardcodees IPs.
- **Secretos:** nada de claves/tokens en el repo. La clave SSH de la app vive
  en `~/Library/Application Support/com.atlas.panel/keys/` (nunca se commitea).
- **Verificación:** `cargo check --workspace`, `cargo test -p atlas-panel`,
  `cargo fmt -p atlas-panel -- --check`, `npm run build:ui` y prueba visual con
  `npm run dev` (comprobar icono de barra, menú y tarjetas). Las pruebas reales
  (`consulta_health_del_relay_real`, `reinicia_el_relay_real`) están ignoradas
  por defecto y se lanzan con `cargo test -p atlas-panel -- --ignored`:
  **reinician el relay de verdad**, así que pide permiso antes.
- **Bundle obligatorio tras validar en dev:** cuando `npm run dev` funcione y
  el usuario lo confirme, ejecuta `npm run build` dentro de `apps/panel-app`
  (Tauri) con la identidad **Apple Development**
  (`APPLE_SIGNING_IDENTITY="Apple Development: …" npm run build`, mirar
  `security find-identity -v -p codesigning`), abre el `.app` generado
  (`target/release/bundle/macos/Hub Panel.app`) y pruébalo. Verifica la
  firma profunda (`codesign --verify --deep --strict --verbose=2`) antes de
  dar la fase por terminada. No basta con que funcione en dev.
- **Instalación local:** para uso real y para probar el autoarranque, copia el
  bundle con `ditto "target/release/bundle/macos/Hub Panel.app"
  "$HOME/Applications/Hub Panel.app"` y lánzalo desde `~/Applications`. No
  dejes el autoarranque apuntando a `target/release` (se puede limpiar).
- **Autoarranque al login:** plugin `tauri-plugin-autostart` con LaunchAgent
  (`~/Library/LaunchAgents/Hub Panel.plist`, apunta a la ruta del `.app`).
  Se activa en el primer arranque del bundle y se marca con
  `autostart-initialized` en `~/Library/Application Support/com.atlas.panel/`;
  el menú de la barra permite desactivarlo y **no** se vuelve a activar solo.
  En dev está deshabilitado a propósito (no registra `target/debug`). Si mueves
  la app de carpeta, desactiva y reactiva el autoarranque desde el menú.

## Reglas de construcción y pruebas

### Antes de cambiar código
- Lee el código y la documentación directamente relacionados con el cambio.
- Ejecuta `git status --short` y conserva los cambios ajenos.
- Usa `rg` para localizar usos antes de retirar una API, comando IPC, dependencia o contrato.
- No recorras ni reestructures todo el repositorio si el cambio no lo requiere.

### Arquitectura
- React + Three.js → presentación, interacción, estados visuales.
- Rust + Tauri → capacidades nativas, IPC, permisos, persistencia, coordinación.
- Contratos tipados para eventos, comandos, errores y estados compartidos.
- Proveedores de IA detrás de interfaces reemplazables.
- Cada herramienta/integración en un adaptador aislado y desactivable.
- El frontend nunca conoce claves, tokens ni detalles internos de proveedores.

### Seguridad y permisos
- Nunca guardar secretos/tokens/contraseñas/claves en el repositorio.
- Validar entradas antes de ejecutar herramientas; normalizar respuestas externas.
- Confirmación explícita para escritura, comunicación externa, ejecución de comandos, cambios de configuración y borrado.
- La falta de respuesta nunca significa autorización.
- Cancelación, timeouts y estados de error en operaciones lentas o con efectos.
- No exponer secretos, rutas privadas, datos personales ni contenido sensible en logs/respuestas.

### TypeScript y Rust
- TS estricto; no rebajar configuración del compilador.
- Rust modular; errores estructurados vía IPC.
- Evitar `unwrap`/`expect` en rutas controladas por el usuario o dependientes del entorno.
- Versionar eventos públicos, payloads y transiciones de estado.
- Preferir interfaces pequeñas, funciones acotadas, dependencias explícitas.
- Evitar estado global mutable si una sesión o módulo puede ser su propietario.

### Pruebas
- Ejecuta, cuando estén disponibles: `npm run build`, `npm run lint`, `cargo check --workspace`.
- Valida comandos IPC con entradas válidas, inválidas, vacías y canceladas.
- Comprueba manualmente los estados visuales afectados por un cambio de frontend.
- Verifica cancelación, errores y permisos en cualquier flujo que ejecute acciones.
- Si cambia el bundle de macOS, regenera con la identidad Apple Development y verifica firma profunda.
- Simula proveedores de IA y herramientas externas; no dependas de APIs reales para pruebas unitarias.
- No consideres terminado un cambio solo porque compile: debe respetar contratos y documentación.
- **Testing con el usuario:** lanza `npm run dev` y prueba contra http://localhost:5173 junto con el usuario.
- **Verificación en producción:** después de validar en dev, ejecuta `npm run tauri build` y prueba la aplicación generada.
- **Si el cambio afecta a `server/`:** despliega el relay con `./scripts/deploy-relay.sh` y verifica `/v1/health`.
- **Registro obligatorio:** documenta todo lo que se hace (pruebas ejecutadas, resultados, despliegues) en los documentos pertinentes.

### Documentación
- Actualiza la documentación cuando cambien arquitectura, contratos, permisos o comportamiento visible.
- Añade implementaciones terminadas a `docs/implemented.md` con el nombre exacto del commit.
- Mantén `docs/implementation-plan.md` alineado con el estado real.
- **Registro de trabajo:** cada cambio realizado (pruebas, despliegues, decisiones) debe quedar documentado en los documentos pertinentes. Este registro es importante y debe mantenerse actualizado.

### Commits y publicación
- Crea un commit únicamente cuando el usuario dé permiso explícito.
- Cuando se conceda, publica inmediatamente en el remoto de GitHub configurado.
- No hagas push, reescribas el historial ni cambies de rama sin permiso explícito.
- Si el cambio afecta a `server/`, despliega con `./scripts/deploy-relay.sh`.
