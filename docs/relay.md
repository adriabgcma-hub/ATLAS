# Relay — manual de operaciones (equipo compartido)

Documento de referencia para **la IA del Mac de desarrollo** (`adris-macbook`,
macOS) y cualquier colaborador externo: cómo funciona el relay, dónde vive, qué
tocar y qué NO tocar. Objetivo: estabilidad del server sin sorpresas.

## 1. Reparto de máquinas

| Máquina | Rol | Qué corre ahí |
|---|---|---|
| `adris-macbook` (macOS) | **desarrollo** | repo Git, frontend Tauri (`npm run dev` → 5173), commits/push, build de app |
| `atlas-hub` (Debian 13 «trixie», MacBookPro12,1 de 2015) | **servidor** | repo Git en `/opt/atlas/repo` (solo pull en deploy), relay `atlas-relay.service` (systemd de sistema) |

`atlas-hub` es el mismo MacBookPro12,1 reinstalado de cero con un esquema
nuevo; su IP puede cambiar, así que en URLs y comandos se usa SIEMPRE el
nombre (`http://atlas-hub.local:8765`), nunca la IP. SSH del Mac de desarrollo
→ `atlas-hub`: alias `atlas-hub` (usuario **`adria_admin`**, administrador,
solo con clave, `~/.ssh/id_ed25519`).

El usuario del relay es **`atlas`** (usuario de sistema, home `/opt/atlas`,
sin login ni sudo): no se entra con él. El despliegue automatizado usa un sudo
sin contraseña y mínimo (`deploy/server/atlas-deploy.sudoers`): pull como
`atlas` y restart/is-active/status del servicio como root. Nada más.

Ojo: `deploy-relay.sh` se ejecuta desde el Mac de desarrollo y entra por SSH
en `atlas-hub` para hacer `sudo -u atlas git pull` + `sudo systemctl restart
atlas-relay`. Nunca edites el código del repo desde `atlas-hub` a mano: lo que
corresponde es commitear en el Mac y desplegar.

## 2. Anatomía del relay

- **Un solo archivo normalizado**: `server/atlas_relay.py` (~1300 líneas) +
  `server/system_monitor.py` (panel). Solo stdlib, Python ≥ 3.11
  (`python3.13` de Debian). Sin pip, sin venv, sin dependencias — así para siempre.
- **HTTP**: `http.server.ThreadingHTTPServer` escuchando EXCLUSIVAMENTE la IP
  LAN de `atlas-hub` (`RELAY_HOST` vacío = IP LAN autodetectada), puerto 8765;
  no 0.0.0.0, no loopback. Perímetro = red local /24 (`same_network_only`):
  cualquier otra IP → `403`. NO hay auth HTTP: el perímetro es la red.
  `127.0.0.1` también pasa el filtro `/24` (probes locales), pero como no
  escucha en loopback, desde el propio servidor se consulta vía IP LAN o
  `atlas-hub.local`.
- **Red**: `ufw` en `atlas-hub` deja pasar `8765/tcp` SOLO desde
  `192.168.100.0/24`. Nada abierto a internet. **Estado real (2026-10):** la
  unidad `ufw` está instalada y `enabled`, pero **INACTIVA**; mientras se
  activa (requiere sudo interactivo), el perímetro operativo es el filtro
  `same_network_only` del relay más el bind a la IP LAN.
- **Servicio**: systemd **de sistema**: `atlas-relay.service`
  (`/etc/systemd/system/atlas-relay.service`; fuente versionada en
  `deploy/server/atlas-relay.service`), `User=atlas`,
  `WorkingDirectory=/opt/atlas/repo`, `Restart=on-failure`,
  `After=network-online.target` + `Wants=network-online.target`, endurecido
  con `NoNewPrivileges=true`, `ProtectSystem=strict`, `ProtectHome=true`,
  `PrivateTmp=true` y `ReadWritePaths=/opt/atlas/repo/temp`. Arranca con el
  sistema (`multi-user.target`): no hay linger que gestionar.
- **Despliegue**: el repo en el servidor se clonó por HTTPS (público, sin
  credenciales) y es dueño de `atlas:atlas`; el Mac de desarrollo commitea y
  hace push, y el deploy hace pull como `atlas` vía el sudoers mínimo.
- **Estado en disco**: `temp/atlas-relay/` (git-ignored): `sessions.json`
  (sesiones+mensajes+seq, escritura atómica), `telegram-offset.json` (offset
  del long-poll), `relay.log` (log detallado por categorías). Se puede borrar
  sin drama (pierdes historial), pero no lo borres «para probar».
- **Arranque**: `load_dotenv()` lee `/opt/atlas/repo/.env` (dueño `atlas`,
  modo 600; NUNCA en Git). Reinicio: los estados de sesión no terminales
  vuelven a `idle`; eventos vivos se pierden (el seq sobrevive en disco para
  no cegar a los clientes).

## 3. Ciclo de una petición

1. `POST /v1/send` (app) o mensaje Telegram aprobado → `Brain.receive()`:
   user message + evento `state: thinking` + **cola FIFO** (1 worker único).
2. Worker → Groq (máx 4 turnos, modelo por defecto `openai/gpt-oss-120b`,
   claves rotativas `GROQ_API_KEY_1..3`).
3. Tools: `calendar_read` (auto) y `calendar_write` (**confirmación**:
   tarjeta en la app y/o DM Telegram; texto libre `sí/adelante` o `no`;
   timeout 300 s). Los calendarios editables se limitan con
   `CALDAV_EDIT_CALENDARS` (defecto Exámenes,Celebraciones,Horario,Deberes).
4. Éxito → estado `completed` + mensaje assistant + respuesta por Telegram si
   la petición vino de ahí. Error → `failed` (`timeout`, `http_*`,
   `max_turns`, …). Cancelable vía `POST /v1/cancel` (o cancelación en masa
   desde el panel).

Estados de sesión: `idle, thinking, awaiting_confirmation, executing,
responding, completed, failed, cancelled`. Terminales: `idle/completed/failed/cancelled`.

## 4. Telegram

- Long-poll `getUpdates` (`timeout=25`), offset persistido; backoff 5→60 s si
  falla la red.
- Solo acepta mensajes de `TELEGRAM_ALLOWED_USER_IDS` (numéricos, separados por
  comas). Con token configurado y sin este set, el relay **no arranca** (config exit 2).
- El token es de `@BotFather` (`TELEGRAM_BOT_TOKEN`); `getMe` se guarda en
  `/v1/health` → `bot.username` (desde el panel se ve).
- Confirmaciones de escritura se pueden resolver por texto de Telegram → misma
  máquina de confirmación que la tarjeta de la app.

## 5. Endpoints (resumen; detalle en `relay-api.md`)

| Ruta | Qué |
|---|---|
| `GET /v1/health` | ok, telegram, llm, calendar, events, queue, inflight, bot |
| `GET /v1/session` | sesión activa + mensajes + cursor |
| `GET /v1/events?since=N` | feed de eventos (poll 4 s en el front) |
| `GET /v1/logs?tail=N` | log del relay |
| `GET /v1/stats` | tokens consumidos (por modelo) |
| `POST /v1/send /v1/cancel /v1/confirm` | ciclo de conversación |
| `GET /v1/calendar/*` | solo lectura (iCloud CalDAV) |
| `GET /v1/system` | snapshot del host (unidades systemd, puertos, recursos) |
| `POST /v1/system/asks` + `/v1/system/resolve` | acciones systemd con doble confirmación (op-id TTL 30 s) |
| `POST /v1/system/cancel_all` | cancela peticiones en vuelo (también doble confirmación) |
| `GET /panel/*` | estático del panel (build Vite en `apps/panel/dist`) |

CORS multi-origen: `RELAY_CORS_ORIGIN` admite lista separada por comas
(`http://localhost:5173,http://localhost:5174`); se devuelve solo el Origin que
hace la petición si está en la lista. Misma LAN indispensable.

## 6. Panel (v1, `apps/panel/`)

Panel en `/panel/` (build Vite de `apps/panel/dist`), recortado a la realidad
de atlas-hub (2026-10):

- Tarjetas: **Servicios systemd** (ssh, avahi, ufw, atlas-relay), Puertos a
  la escucha, Este Mac (RAM/disco/temp/batería/AC), Relay·Telegram (bot,
  tokens por modelo, cola/vuelo), Peticiones (mensajes con origen TG/local +
  cancelar en vuelo) y Log del relay. Carta ámbar de confirmación para
  acciones.
- **Fuera del panel hasta que existan en atlas-hub** (no se observan, ni se
  piden): docker/contenedores, nube Nextcloud (compose, tailscale serve),
  unidades del servidor antiguo (cups, macfan, lid-monitor, smartmontools,
  NetworkManager) y unidades de usuario (sin linger). Volverán cuando se
  integren de verdad, una a una.
- Acciones systemd = lista blanca `PANEL_ACTION_UNITS`, que por defecto está
  **VACÍA** en atlas-hub: sin reglas polkit para el usuario `atlas`, ninguna
  acción de unidad puede prosperar. Flujo de doble confirmación intacto
  (botón → `asks` op-id → tarjeta ámbar → `resolve allow_once/deny`; sin
  permiso implícito).
- Piezas de host del servidor antiguo (polkit `50-atlas-panel.rules`,
  sudoers `atlas-panel`, ayudante `/usr/local/lib/atlas/panel-docker.sh`)
  NO existen en atlas-hub: se reinstalarán solo cuando se decida qué
  unidades/compose son accionables (ver §7).

## 7. Nube del servidor antiguo (Nextcloud)

> ⚠️ **PENDIENTE DE MIGRAR a `atlas-hub`.** La nube Nextcloud
> (compose + Tailscale) sigue en el servidor antiguo; estos comandos
> (`/home/adria/nextcloud/…`) no valen en `atlas-hub` hasta que se decida la
> migración. No tocar. El panel ya no muestra tarjeta de nube/docker (ver §6);
> si se migra la nube, entonces se reintegrará al panel.

- `~/nextcloud/` con `docker-compose.yml`: servicios `db` (mariadb:10.6) y
  `app` (nextcloud:apache, expuesto solo en `127.0.0.1:8080`). *(En el
  servidor antiguo; en atlas-hub no existe.)*
- Se publica vía **`tailscale serve`**: `https://macbook-nube.tail971f40.ts.net`
  → proxy a `127.0.0.1:8080` (HTTPS intra-tailnet; cert válido, solo tailnet).
- Reglas: NO tocar `nextcloud/data/` a mano (el contenedor maneja uid/perms);
  NO sincronizar esa carpeta al repo Git; restaurar/reiniciar SOLO con
  `docker compose -f /home/adria/nextcloud/docker-compose.yml …`. El `.env`
  del compose carga contraseñas en claro: nunca en Git.

## 8. Operación desde el Mac de desarrollo

```bash
# ver logs del relay (en atlas-hub)
ssh adria_admin@atlas-hub "journalctl -u atlas-relay -f"
# o el log de categorías del cerebro (si los permisos de /opt/atlas lo
# permiten; si no, journalctl)
ssh adria_admin@atlas-hub "tail -f /opt/atlas/repo/temp/atlas-relay/relay.log"

# desplegar relay (¡primero commitear!)
git add -A && git commit -m "feat(relay): …"
./scripts/deploy-relay.sh     # push → git pull como atlas en atlas-hub → sudo systemctl restart → health

# panel: build estático antes de desplegar (lo sirve el relay; PENDIENTE DE MIGRAR)
cd apps/panel && npm install && npm run build
```

El `.env` real solo en `/opt/atlas/repo/.env` (dueño `atlas`, modo 600, nunca
en Git). Para actualizarlo desde el Mac:

```bash
scp .env adria_admin@atlas-hub:/tmp/atlas-env
ssh -t adria_admin@atlas-hub 'sudo install -m 600 -o atlas -g atlas /tmp/atlas-env /opt/atlas/repo/.env && rm -f /tmp/atlas-env'
```

Variables del deploy: `ATLAS_DEPLOY_HOST` (defecto `atlas-hub`) y
`ATLAS_RELAY_URL` (defecto `http://atlas-hub.local:8765/v1/health`). La
instalación inicial del servidor está detallada en §11.

### app ATLAS Panel: clave de reinicio

La app macOS `ATLAS Panel` (`apps/panel-app`, bundle `com.atlas.panel`) puede
reiniciar `atlas-relay` sin sesión interactiva. Usa una clave `ed25519`
exclusiva:

- Local: `~/Library/Application Support/com.atlas.panel/keys/panel_ed25519`
  (`600`, sin passphrase, fuera del repo). Huella
  `SHA256:wExdWnMDtEF/pJCP8+nTbbpjMjFkHgkQ+l/qTZKdcr0` (`atlas-panel`).
- Servidor: una línea en `~adria_admin/.ssh/authorized_keys` con comando
  forzado:

```
command="sudo systemctl restart atlas-relay && sudo systemctl is-active atlas-relay",no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIFxB0/h/yGW5mREh35tI4JWCQFlukIjP2dp5CuvLXx0 atlas-panel
```

- Solo permite reiniciar y consultar `atlas-relay` (sudoers
  `deploy/server/atlas-deploy.sudoers`, sin contraseña). No abre shell ni
  acepta comandos del cliente.
- Probar sin la app:
  `ssh -i "~/Library/Application Support/com.atlas.panel/keys/panel_ed25519" -o IdentitiesOnly=yes atlas-hub panel-restart`
  → imprime `active` y **reinicia el relay** (pedir permiso antes).
- Rotación: generar clave nueva, añadir su línea, probar y borrar la anterior.
  Si se pierde la clave local, basta con generar otra y sustituir la línea.

## 9. Checklist de estabilidad (antes y después de cada deploy)

1. `git status --short` vacío (el deploy aborta si hay cambios).
2. `curl http://atlas-hub.local:8765/v1/health` → `ok:true`, telegram/llm/calendar ok.
3. Después del deploy: repetir health + `GET /v1/session` (cursor no retrocede) +
   una petición local de prueba (sin tools) para ver el ciclo completo.
4. Si tocaste `PANEL_*`: re-probar el panel en red local (sin polkit/sudoers
   de momento en atlas-hub, las acciones siguen sin habilitarse).

### Watchdog del relay (`atlas-health.timer`)

- Instalación (una vez, tras desplegar el relay para que el script esté en
  `/opt/atlas/repo`): `./scripts/deploy-healthcheck.sh` → pide la contraseña de
  `adria_admin`. Deja `atlas-health.service` (oneshot, root) y
  `atlas-health.timer` (90 s tras el arranque, luego cada 60 s).
- Comprueba `/v1/health` en la IP LAN (nunca loopback), timeout 3 s, y reinicia
  `atlas-relay` tras **2 fallos consecutivos**. Contador en
  `/var/lib/atlas-health/failures`; se pone a cero tras cada intento.
- Ver: `systemctl list-timers atlas-health.timer` y
  `journalctl -u atlas-health -n 30`.
- **Mantenimiento:** antes de parar el relay a mano,
  `sudo systemctl stop atlas-health.timer`; después,
  `sudo systemctl start atlas-health.timer`. Si no, el watchdog lo levanta en
  1-2 min.
- Prueba en seco sin tocar el servicio (desde el Mac o el servidor):
  `ATLAS_HEALTH_URL=http://127.0.0.1:9 ATLAS_HEALTH_DRY_RUN=1
  ATLAS_HEALTH_STATE_DIR=/tmp/atlas-health python3
  deploy/server/atlas-healthcheck.py` dos veces → `1/2` y luego
  `DRY RUN: reiniciaría atlas-relay`.
- Verificado el 2026-10-25 en `atlas-hub`: parando `atlas-relay` a las 23:24,
  el watchdog lo reinició a las 23:26:01 y `/v1/health` volvió a 200 a las
  23:26:05. `NRestarts` sigue a 0 porque no cuenta reinicios manuales.

### Prohibiciones explícitas

- NO martillear iCloud CalDAV: cada prueba real contra iCloud requiere el
  permiso explícito del usuario (403 por throttling de Apple si no).
- NO abrir puertos del relay a internet, NO webhooks de Telegram, NO auth vía
  Telegram con permisos ampliados. Perímetro: LAN.
- NO editar `temp/atlas-relay/*` "mientras corre" (la escritura es atómica por
  el proceso; el historial es del usuario).
- NO introducir dependencias en el relay (solo stdlib) ni bindings binarios.
- NO reiniciar `atlas-relay` fuera del flujo del deploy sin avisar (el front
  de Telegram mantiene en vuelo; sesiones ok, pero mejor no).

## 10. Troubleshooting rápido

| Síntoma | Causa probable | Acción |
|---|---|---|
| `403 same_network_only` | cliente fuera de la /24 | conecta a la LAN/VPN correcta |
| Telegram `401/404` en getMe | token inválido | relay muere con exit 2; arreglar token en `/opt/atlas/repo/.env` y redeploy |
| `502 calendar_upstream` / 403 iCloud | throttle de Apple o credenciales | espera 1-2 min; NO reintentar en bucle |
| relay caído y vuelve a subir en bucle | error compilando/arranque | `journalctl -u atlas-relay -n 50` |
| deploy aborta en `sudo -u atlas git …` | falta el sudoers mínimo | instalar `deploy/server/atlas-deploy.sudoers` (ver §11) |
| deploy aborta en `sudo systemctl restart` | unidad no instalada o unit rota | revisar `/etc/systemd/system/atlas-relay.service` (ver §11) |
| health falla sin resolver `atlas-hub.local` | avahi caído u otra subred | revisar avahi o usar la IP del momento con `ATLAS_RELAY_URL` |
| sin respuesta en `127.0.0.1:8765` en el servidor | el relay NO escucha en loopback (correcto) | consultar por la IP LAN o `atlas-hub.local` |
| relay "activo" pero panel/frontal no conectan | arrancó sin red y quedó atado a `127.0.0.1` | `ss -ltn \| grep 8765`; reiniciar cuando haya red (el reintento de 60 s ya lo evita) |
| panel 404 en `/panel/` | falta `apps/panel/dist` (no build) | `cd apps/panel && npm run build` y copia `dist/` al repo del servidor (no va en Git) |
| tarjeta Servicios sin botones de acción | `PANEL_ACTION_UNITS` vacía (por defecto en atlas-hub; faltan reglas polkit) | consciente: instalar reglas polkit para `atlas` y añadir unidades al `.env` |
| acción del panel `action_not_allowed` | unidad fuera de lista blanca | añadir a `PANEL_ACTION_UNITS` (solo con polkit listo y siendo consciente) |

## 11. Instalación en atlas-hub (servidor nuevo)

Estado previo (ya así en el servidor): hostname `atlas-hub`; `adria_admin`
administrador con la clave pública del Mac en `authorized_keys` (acceso SSH
solo con clave); usuario de sistema `atlas` (home `/opt/atlas`, sin login ni
sudo) que posee `/opt/atlas/repo` (repo público clonado por HTTPS); `ufw`
activo. Todos los comandos se ejecutan **en atlas-hub** como `adria_admin`;
el sudo de la instalación es interactivo (con contraseña), mientras que el
sudo sin contraseña del sudoers.d solo sirve al despliegue automatizado
posterior (pull + restart + is-active/status, nada más).

```bash
# En el Mac de desarrollo, SOLO si el .env aún no está en el servidor:
scp .env adria_admin@atlas-hub:/tmp/atlas-env
ssh -t adria_admin@atlas-hub 'sudo install -m 600 -o atlas -g atlas /tmp/atlas-env /opt/atlas/repo/.env && rm -f /tmp/atlas-env'

# En atlas-hub (ssh adria_admin@atlas-hub):

# 1. Validar el sudoers ANTES de instalarlo (error de sintaxis → abortar)
sudo visudo -cf /opt/atlas/repo/deploy/server/atlas-deploy.sudoers

# 2. Copiar la unidad del servicio y el sudoers
sudo install -m 644 -o root -g root /opt/atlas/repo/deploy/server/atlas-relay.service /etc/systemd/system/atlas-relay.service
sudo install -m 440 -o root -g root /opt/atlas/repo/deploy/server/atlas-deploy.sudoers /etc/sudoers.d/atlas-deploy
sudo visudo -cf /etc/sudoers.d/atlas-deploy

# 3. Directorio de estado (dueño atlas; con ProtectSystem=strict es la única
#    ruta escribible del servicio y debe existir antes del primer arranque)
sudo install -d -o atlas -g atlas /opt/atlas/repo/temp

# 4. Registrar y arrancar
sudo systemctl daemon-reload
sudo systemctl enable --now atlas-relay
systemctl is-active atlas-relay        # debe decir: active

# 5. ufw: 8765/tcp SOLO desde la /24 (nada a internet)
sudo ufw allow from 192.168.100.0/24 to any port 8765 proto tcp comment 'ATLAS relay'
sudo ufw status numbered

# 6. Verificación (NO vale 127.0.0.1: el relay no escucha en loopback)
ss -ltn | grep 8765                    # a la escucha en la IP LAN
curl http://atlas-hub.local:8765/v1/health
```

Y desde el Mac de desarrollo, comprobación final + primer despliegue
automatizada (a partir de aquí los despliegues van sin contraseña, gracias
solo al sudoers mínimo):

```bash
curl http://atlas-hub.local:8765/v1/health   # ok:true, telegram/llm/calendar
./scripts/deploy-relay.sh --pull             # sudo -u atlas pull + sudo systemctl restart + health
```

Notas de la instalación:

- Si falta un token obligatorio (p. ej. Telegram bien configurado pero sin
  `TELEGRAM_ALLOWED_USER_IDS`), el relay muere con exit 2 en el arranque:
  verlo con `journalctl -u atlas-relay -n 50`.
- `Wants/After=network-online.target` es best-effort: si la red tarda y el
  relay no halla la IP LAN al arrancar, **reintenta la autodetección hasta
  60 s** (2 s entre intentos); si sigue en loopback, sale con error de
  config (exit 2) para que `Restart=on-failure` lo reintente. Así nunca se
  queda escuchando en `127.0.0.1` (panel/frontal inalcanzables).
- El endurecimiento incluye `NoNewPrivileges=true`: correcto porque el panel
  ya no usa sudo (no hay ayudante docker). Ver §6 para cómo se reintroducirán
  acciones cuando se integren unidades.
