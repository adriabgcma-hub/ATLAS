# Relay — manual de operaciones (equipo compartido)

Documento de referencia para **la IA del Mac de desarrollo** (`adris-macbook`,
macOS) y cualquier colaborador externo: cómo funciona el relay, dónde vive, qué
tocar y qué NO tocar. Objetivo: estabilidad del server sin sorpresas.

## 1. Reparto de máquinas

| Máquina | Rol | Qué corre ahí |
|---|---|---|
| `adris-macbook` (macOS, otro Mac) | **desarrollo** | repo Git, frontend Tauri (`npm run dev` → 5173), commits/push, build de app |
| `macbook-nube` = `homelab` (192.168.100.26, Debian 13 «trixie») | **servidor** | repo Git (solo lectura + deploy), relay `atlas-relay.service`, nube Nextcloud, Tailscale, panel |

`homelab` es el **MacBookPro12,1** de 2015 con Debian. Sí: el "servidor" es un
MacBook viejo con Linux. SSH de `adris-macbook` → `homelab`: clave
`~/.ssh/atlas_linux`, usuario `adria`.

Ojo: `deploy-relay.sh` se ejecuta **desde el Mac de desarrollo** y entra por SSH
en `homelab` para hacer `git pull` + `systemctl --user restart atlas-relay`.
Nunca edites el código del repo desde `homelab` a mano: lo que corresponde es
commitear en el Mac y desplegar.

## 2. Anatomía del relay

- **Un solo archivo normalizado**: `server/atlas_relay.py` (~1300 líneas) +
  `server/system_monitor.py` (panel). Solo stdlib, Python ≥ 3.11
  (`python3.13` de Debian). Sin pip, sin venv, sin dependencias — así para siempre.
- **HTTP**: `http.server.ThreadingHTTPServer` escuchando EXCLUSIVAMENTE la IP
  LAN (`192.168.100.26:8765`; no 0.0.0.0, no loopback). Perímetro = red local /24
  (`same_network_only`): cualquier otra IP → `403`. NO hay auth HTTP: el perímetro
  es la red. `127.0.0.1` también pasa (para probes locales).
- **Servicio**: systemd **de usuario**: `atlas-relay.service`
  (`~/.config/systemd/user/atlas-relay.service`), `Restart=on-failure`,
  `WorkingDirectory=%h/ATLAS`, con **Linger=yes** (sobrevive a cierres de sesión).
- **Estado en disco**: `temp/atlas-relay/` (git-ignored):
  `sessions.json` (sesiones+mensajes+seq, escritura atómica), `telegram-offset.json`
  (offset del long-poll), `relay.log` (log detallado por categorías). Se puede
  borrar sin drama (pierdes historial), pero no lo borres «para probar».
- **Arranque**: `load_dotenv()` lee `ATLAS/.env` (NUNCA en Git). Reinicio: los
  estados de sesión no terminales vuelven a `idle`; eventos vivos se pierden
  (el seq sobrevive en disco para no cegar a los clientes).

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

- React 19 + Vite base `/panel/`, **sin router**; tokens copiados de
  `src/App.css` del frontal (dark glass + cian).
- Tarjetas: Servicios systemd (con start/stop/restart), Puertos, Docker,
  Nube Nextcloud (con compose restart/stop/start), Este Mac (RAM/disco/temp/
  batería/tapa), Relay+Telegram (bot username, tokens, cola/vuelo), Peticiones
  (mensajes con origen TG/local + cancelar en vuelo), Log del relay.
- Acciones = flujo de **doble confirmación**: botón → `asks` (op-id) → tarjeta
  ámbar → `resolve allow_once/deny`. Sin permiso implícito: la falta de
  respuesta nunca autoriza (regla ATLAS).
- Permiso del host (una vez, ya instalado en homelab):
  - polkit `/etc/polkit-1/rules.d/50-atlas-panel.rules` → `adria` solo gestiona
    las unidades de la lista (docker, cups, avahi, macfan, lid-monitor,
    smartmontools). `sshd`/`tailscaled`/`NetworkManager` EXCLUIDOS a propósito.
  - sudo `/etc/sudoers.d/atlas-panel` → `adria` sin contraseña SOLO para
    `/usr/local/lib/atlas/panel-docker.sh` (script con raíz fija: `ps` y
    compose start/stop/restart de app/db; cualquier otro argumento → banned).
- Variables `PANEL_*` (defaults en `system_monitor.py`; ver `relay-api.md`).

## 7. Nube de homelab (Nextcloud)

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
# ver logs del relay (en homelab)
ssh -i ~/.ssh/atlas_linux adria@192.168.100.26 "journalctl --user -u atlas-relay -f"
# o el log de categorías del cerebro
ssh -i ~/.ssh/atlas_linux adria@192.168.100.26 "tail -f ATLAS/temp/atlas-relay/relay.log"

# desplegar relay (¡ primero commitear! nothing else)
git add -A && git commit -m "feat(relay): …"
./scripts/deploy-relay.sh     # push → git pull en homelab → restart → health

# panel: build estático antes de desplegar (lo sirve el relay)
cd apps/panel && npm install && npm run build
```

Durante el dev en homelab o para probar el panel con otro editor:
`cd ATLAS && npm run dev` (front 5173), `cd apps/panel && npm run dev` (panel
5174). El `.env` de homelab ya incluye ambos orígenes en `RELAY_CORS_ORIGIN`.

## 9. Checklist de estabilidad (antes y después de cada deploy)

1. `git status --short` vacío (el deploy aborta si hay cambios).
2. `curl http://192.168.100.26:8765/v1/health` → `ok:true`, telegram/llm/calendar ok.
3. Después del deploy: repetir health + `GET /v1/session` (cursor no retrocede) +
   una petición local de prueba (sin tools) para ver el ciclo completo.
4. Si tocaste `PANEL_*`/polkit/sudoers: re-probar una acción desde el panel con red local.

### Prohibiciones explícitas

- NO martillear iCloud CalDAV: cada prueba real contra iCloud requiere el
  permiso explícito del usuario (403 por throttling de Apple si no).
- NO abrir puertos del relay a internet, NO webhooks de Telegram, NO auth vía
  Telegram con permisos ampliados. Perímetro: LAN + Tailnet.
- NO editar `temp/atlas-relay/*` "mientras corre" (la escritura es atómica por
  el proceso; el historial es del usuario).
- NO introducir dependencias en el relay (solo stdlib) ni bindings binarios.
- NO reiniciar `atlas-relay` fuera del flujo del deploy sin avisar (el front
  de Telegram mantiene en vuelo; sesiones ok, pero mejor no).

## 10. Troubleshooting rápido

| Síntoma | Causa probable | Acción |
|---|---|---|
| `403 same_network_only` | cliente fuera de la /24 | conecta a la LAN/VPN correcta |
| Telegram `401/404` en getMe | token inválido | relay muere con exit 2; arreglar token en `.env` y redeploy |
| `502 calendar_upstream` / 403 iCloud | throttle de Apple o credenciales | espera 1-2 min; NO reintentar en bucle |
| relay caído y up-de nuevo en bucle | error compilando/arranque | `journalctl --user -u atlas-relay -n 50` |
| panel 404 en `/panel/` | falta `apps/panel/dist` (no build) | `cd apps/panel && npm run build` y redeploy |
| docker «sin acceso» en panel | sudoers del ayudante se perdió | re-instalar `/usr/local/lib/atlas/panel-docker.sh` + sudoers según §6 |
| acción del panel `action_not_allowed` | unidad fuera de whitelist | editar `PANEL_ACTION_UNITS`/polkit ( conscious) |
