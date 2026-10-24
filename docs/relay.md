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
  `192.168.100.0/24`. Nada abierto a internet.
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
| `GET /v1/system` | snapshot del host (unidades, puertos, docker, nube, recursos) |
| `POST /v1/system/asks` + `/v1/system/resolve` | acciones systemd/compose con doble confirmación (op-id TTL 30 s) |
| `POST /v1/system/cancel_all` | cancela peticiones en vuelo (también doble confirmación) |
| `GET /panel/*` | estático del panel (build Vite en `apps/panel/dist`) |

CORS multi-origen: `RELAY_CORS_ORIGIN` admite lista separada por comas
(`http://localhost:5173,http://localhost:5174`); se devuelve solo el Origin que
hace la petición si está en la lista. Misma LAN indispensable.

## 6. Panel (v1, `apps/panel/`)

> ⚠️ **PENDIENTE DE MIGRAR al servidor `atlas-hub`.** El panel y sus permisos
> de host (polkit + sudoers del ayudante docker) están instalados en el
> servidor antiguo; nada de esto existe todavía en `atlas-hub`. Además, el
> endurecimiento de la unidad nueva (`NoNewPrivileges=true`) bloquea `sudo`
> dentro del relay, con lo que las acciones docker quedarían inoperativas
> hasta revisarse. No toques estas piezas hasta decidir la migración.

- React 19 + Vite base `/panel/`, **sin router**; tokens copiados de
  `src/App.css` del frontal (dark glass + cian).
- Tarjetas: Servicios systemd (con start/stop/restart), Puertos, Docker,
  Nube Nextcloud (con compose restart/stop/start), Este Mac (RAM/disco/temp/
  batería/tapa), Relay+Telegram (bot username, tokens, cola/vuelo), Peticiones
  (mensajes con origen TG/local + cancelar en vuelo), Log del relay.
- Acciones = flujo de **doble confirmación**: botón → `asks` (op-id) → tarjeta
  ámbar → `resolve allow_once/deny`. Sin permiso implícito: la falta de
  respuesta nunca autoriza (regla ATLAS).
- Permiso del host (una vez, ya instalado en el servidor antiguo):
  - polkit `/etc/polkit-1/rules.d/50-atlas-panel.rules` → `adria` solo gestiona
    las unidades de la lista (docker, cups, avahi, macfan, lid-monitor,
    smartmontools). `sshd`/`tailscaled`/`NetworkManager` EXCLUIDOS a propósito.
  - sudo `/etc/sudoers.d/atlas-panel` → `adria` sin contraseña SOLO para
    `/usr/local/lib/atlas/panel-docker.sh` (script con raíz fija: `ps` y
    compose start/stop/restart de app/db; cualquier otro argumento → banned).
- Variables `PANEL_*` (defaults en `system_monitor.py`; ver `relay-api.md`).

## 7. Nube del servidor antiguo (Nextcloud)

> ⚠️ **PENDIENTE DE MIGRAR al servidor `atlas-hub`.** La nube Nextcloud
> (compose + Tailscale) sigue en el servidor antiguo; estos comandos
> (`/home/adria/nextcloud/…`) no valen en `atlas-hub` hasta que se decida la
> migración. No tocar.

- `~/nextcloud/` con `docker-compose.yml`: servicios `db` (mariadb:10.6) y
  `app` (nextcloud:apache, expuesto solo en `127.0.0.1:8080`).
- Se publica vía **`tailscale serve`**: `https://macbook-nube.tail971f40.ts.net`
  → proxy a `127.0.0.1:8080` (HTTPS intra-tailnet; cert válido, solo tailnet).
- Reglas: NO tocar `nextcloud/data/` a mano (el contenedor maneja uid/perms);
  NO sincronizar esa carpeta al repo Git; restaurar/reiniciar SOLO con
  `docker compose -f /home/adria/nextcloud/docker-compose.yml …` (o el panel,
  que lo hace igual con la doble confirmación). El `.env` del compose carga
  contraseñas en claro: nunca en Git.

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

## 9. Checklist de estabilidad (antes y después de cada deploy)

1. `git status --short` vacío (el deploy aborta si hay cambios).
2. `curl http://atlas-hub.local:8765/v1/health` → `ok:true`, telegram/llm/calendar ok.
3. Después del deploy: repetir health + `GET /v1/session` (cursor no retrocede) +
   una petición local de prueba (sin tools) para ver el ciclo completo.
4. Si tocaste `PANEL_*`/polkit/sudoers: re-probar una acción desde el panel con red local.

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
| sin respuesta en `127.0.0.1:8765` en el servidor | el relay NO escucha en loopback | consultar por la IP LAN o `atlas-hub.local` |
| panel 404 en `/panel/` | falta `apps/panel/dist` (no build) | `cd apps/panel && npm run build` y redeploy |
| docker «sin acceso» en panel | panel sin migrar + `NoNewPrivileges` bloquea sudo | leer los avisos de §6 y de `deploy/server/atlas-relay.service` |
| acción del panel `action_not_allowed` | unidad fuera de whitelist | editar `PANEL_ACTION_UNITS` (consciente) |

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
  relay no halla la IP LAN, `Restart=on-failure` lo reintenta.
- El endurecimiento incluye `NoNewPrivileges=true`: las acciones docker del
  panel NO funcionarán (el ayudante `sudo -n` se bloquea). Leer el aviso
  dentro de la unidad y §6 antes de migrar el panel.
