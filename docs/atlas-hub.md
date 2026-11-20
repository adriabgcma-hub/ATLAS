# Atlas Hub — manual del servidor

Documento único de `atlas-hub`: la máquina, la red, los servicios y cómo se
opera desde el Mac de desarrollo. Sustituye a los antiguos `relay.md` y
`atlas-hub-plan.md`. **Para la IA del Mac y cualquier colaborador: leer esto
antes de tocar el servidor.** La API HTTP del relay está en
[`relay-api.md`](./relay-api.md); la app de barra de menús en
[`panel-app-plan.md`](./panel-app-plan.md).

Estado (2026-11-20): relay, Nextcloud, Jellyfin, Cockpit, Tailscale y `ufw`
operativos; watchdog del relay y backup semanal a microSD instalados y
verificados. Pendientes en §12.

## 1. Reparto de máquinas

| Máquina | Rol | Qué corre ahí |
|---|---|---|
| `adris-macbook` (macOS) | **desarrollo** | repo Git, frontend Tauri (`npm run dev` → 5173), commits/push, builds de app |
| `atlas-hub` (Debian 13 «trixie») | **servidor** | repo en `/opt/atlas/repo` (solo pull), relay, servicios Podman, Cockpit, Tailscale |

- **Nombre, nunca IP:** la IP cambia; en URLs y comandos se usa siempre
  `atlas-hub.local` (mDNS/avahi). Si falla la resolución, ver §10.
- **SSH:** alias `atlas-hub` del Mac → usuario **`adria_admin`** (administrador,
  solo clave, `~/.ssh/id_ed25519`).
- El repo del servidor se clonó por HTTPS (público, sin credenciales) y es
  dueño de `atlas:atlas`. **No se edita a mano en el servidor:** se commitea en
  el Mac y se despliega (§8).

## 2. Hardware y sistema

- MacBookPro12,1 (2015): i5-5257U (2 núcleos/4 hilos), **sin VT-x**, 8 GB RAM +
  5,8 GB swap, **SSD Apple de 128 GB** (106 GB `/` ext4; ~92 GB libres),
  conexión por **WiFi** (`wlp3s0`).
- Debian 13.7, kernel 6.12, systemd con **cgroup v2**, repos Debian:
  `podman` 5.4.2, `cockpit`; `tailscale` desde upstream (no está en repos).
- Disco único sin redundancia → el backup semanal (§9) es la única copia;
  una tarjeta sola no protege ante fallo de disco, por eso se rotan 2.
- Jellyfin va en **direct play** (el CPU no da para transcodificar).

**Decisiones de diseño (por qué):**

| Decisión | Motivo |
|---|---|
| **Tailscale gestionado** | Cero puertos abiertos y cero tocar el router (requisito). Trade-off aceptado: la topología la ve Tailscale; el tráfico va cifrado. |
| **Podman rootless + Quadlet** | Sin daemon root; cada servicio es una unidad systemd; cgroup v2 ya activo. |
| **Cockpit + Hub Panel** | Cockpit aporta terminal/gestión systemd (PAM, maduro); Hub Panel solo resume y enlaza. |
| **MariaDB para Nextcloud** | Evita migrar de SQLite después. |
| **HTTP sobre el túnel** | WireGuard ya cifra; TLS propio queda como mejora (§12). |
| **Todo en el SSD** | Sin disco externo por ahora; media fuera cuando crezca. |

```
atlas-hub (Debian 13 · 8 GB · SSD 128 GB · WiFi)
│
├─ atlas-relay.service   → relay Atlas (8765) + Hefesto (mTLS 8767)
├─ tailscaled            → cliente VPN (sin puertos abiertos)
├─ cockpit.socket        → consola web (9090)
├─ podman rootless (usuario «services», sin login, con linger)
│   ├─ mariadb    (Quadlet) → BD de Nextcloud (no publicada)
│   ├─ nextcloud  (Quadlet) → archivos + CalDAV (8080)
│   └─ jellyfin   (Quadlet) → media direct play (8096)
├─ atlas-health.timer    → watchdog del relay (cada 60 s)
├─ atlas-backup.timer    → backup semanal a microSD (§9)
└─ ufw (activo)

MacBook / iPhone / iPad
└─ cliente Tailscale → red privada 100.64.0.0/10 (misma cuenta)
```

## 3. Red y perímetro

Regla de oro: **no se abre ningún puerto a internet**. Se llega solo desde la
LAN y desde la tailnet.

- **LAN:** `192.168.100.0/24`.
- **Tailscale:** red mesh gestionada, tailnet `100.64.0.0/10`
  (`tail971f40.ts.net`), MagicDNS (`atlas-hub.<tailnet>.ts.net`). Cliente
  `tailscaled` en el hub, el Mac y el iPhone con la misma cuenta. **Mantenerlo
  parcheado y revisar la versión en cada mantenimiento.**
- **Firewall:** `ufw` activo con reglas versionadas e idempotentes en
  `deploy/atlas-hub/ufw.sh` (LAN + tailnet, nada más):

| Puerto | Servicio |
|---|---|
| 22 | SSH |
| 8765 | relay |
| 8767 | Hefesto (mTLS) |
| 9090 | Cockpit |
| 8080 | Nextcloud |
| 8096 | Jellyfin |

- **Perímetro del relay:** además del `ufw`, el relay aplica
  `same_network_only` (LAN + redes de `RELAY_ALLOW_NETWORKS` → `403` para el
  resto; no hay auth HTTP: el perímetro es la red) y escucha en la IP LAN y en
  la de Tailscale (`RELAY_EXTRA_HOSTS`), nunca en `0.0.0.0` ni en loopback.
- **Acceso a los servicios:** LAN → `http://atlas-hub.local:<puerto>`; desde
  fuera → por el nombre MagicDNS del hub sobre la tailnet (`tailscale serve`
  publica 8080/8096 en HTTPS sobre el túnel). Si un servicio está caído,
  `tailscale serve` devuelve **502**.

## 4. Usuarios, claves y permisos

| Usuario | Tipo | Uso |
|---|---|---|
| `adria_admin` | administrador | SSH (solo clave) y `sudo` interactivo; Cockpit autentica por PAM con esta cuenta |
| `atlas` | sistema | dueño de `/opt/atlas/repo`; ejecuta `atlas-relay.service`. Sin login ni sudo |
| `services` | sistema (uid 997) | home `/opt/services`; Podman rootless con `linger`. Sin login |

- **Contraseñas:** el hub usa **una sola contraseña** para servidor,
  Cockpit, Nextcloud (`adria-admin`) y Jellyfin; el Mac tiene la suya, aparte.
  Las contraseñas no se escriben en el repo.
- **Ojo CalDAV:** el relay guarda su credencial de Nextcloud en
  `/opt/atlas/repo/.env` (`CALDAV_APP_PASSWORD`). Si cambia la contraseña de
  `adria-admin`, el calendario falla con `calendar_upstream` (401): re-apuntar
  con `sudo bash /opt/atlas/repo/deploy/atlas-hub/set-nextcloud-env.sh` (valida
  contra Nextcloud y reescribe `CALDAV_*` y las sondas `PANEL_*`). Lo robusto
  es una **contraseña de aplicación** de Nextcloud, no la de la cuenta.
- **Sudoers mínimo** (`deploy/server/atlas-deploy.sudoers`, instalado en
  `/etc/sudoers.d/atlas-deploy`): `adria_admin` puede, **sin contraseña y sin
  nada más**, exactamente:

```
adria_admin ALL=(atlas) NOPASSWD: /usr/bin/git -C /opt/atlas/repo pull --ff-only
adria_admin ALL=(root) NOPASSWD: /usr/bin/systemctl restart atlas-relay, /usr/bin/systemctl is-active atlas-relay, /usr/bin/systemctl status atlas-relay
```

  Cualquier otro `sudo` (instalaciones, backup, ufw…) pide la contraseña de
  `adria_admin` de forma interactiva.

### Clave de reinicio de Hub Panel

La app macOS `Hub Panel` (`apps/panel-app`, bundle `com.atlas.panel`) reinicia
`atlas-relay` sin sesión interactiva con una clave `ed25519` exclusiva:

- Local: `~/Library/Application Support/com.atlas.panel/keys/panel_ed25519`
  (`600`, sin passphrase, fuera del repo). Huella
  `SHA256:wExdWnMDtEF/pJCP8+nTbbpjMjFkHgkQ+l/qTZKdcr0` (comentario `atlas-panel`).
- Servidor: una línea en `~adria_admin/.ssh/authorized_keys` con comando
  forzado:

```
command="sudo systemctl restart atlas-relay && sudo systemctl is-active atlas-relay",no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIFxB0/h/yGW5mREh35tI4JWCQFlukIjP2dp5CuvLXx0 atlas-panel
```

- Solo permite reiniciar y consultar `atlas-relay`; no abre shell.
- Probar sin la app (reinicia el relay de verdad: pedir permiso antes):
  `ssh -i "~/Library/Application Support/com.atlas.panel/keys/panel_ed25519" -o IdentitiesOnly=yes atlas-hub panel-restart`.
- Rotación: generar clave nueva, añadir su línea, probar y solo entonces borrar
  la anterior. Instalarla/rotarla requiere permiso explícito del usuario.

## 5. Servicios autoalojados (Podman rootless + Quadlet)

- **Unidades** versionadas en `deploy/atlas-hub/quadlet/` (`mariadb`,
  `nextcloud`, `jellyfin` + red `atlas-services.network`). `setup-services.sh`
  (idempotente, root) las instala en
  `/opt/services/.config/containers/systemd/`, crea secretos si faltan, baja
  imágenes y arranca todo. Tras cambiar un quadlet: commit → pull en el hub →
  re-ejecutar `sudo bash /opt/atlas/repo/deploy/atlas-hub/setup-services.sh`.
- **Arranque automático:** los quadlets llevan
  `[Install] WantedBy=default.target` y el usuario `services` tiene `linger`.
  `systemctl --user enable` **falla a propósito** («transient or generated»,
  son unidades generadas): comprobar con
  `sudo -u services env HOME=/opt/services XDG_RUNTIME_DIR=/run/user/997 systemctl --user list-dependencies default.target`.
- **Volúmenes:** `mariadb-data`, `nextcloud-config`, `nextcloud-data`,
  `jellyfin-config` y `jellyfin-cache` (cache regenerable). `media` de Jellyfin
  se monta de `/opt/services/media` en solo lectura.
- **Secretos** (fuera de Git): `/opt/services/secrets/mariadb.env` y
  `nextcloud.env` (600, dueño `services`).
- **Nextcloud:** `linuxserver/nextcloud` con MariaDB; `trusted_domains` los fija
  `setup-services.sh` (`localhost`, `atlas-hub`, `atlas-hub.local`, IP de
  Tailscale). Reset de la contraseña de `adria-admin`:
  `sudo bash deploy/atlas-hub/reset-nextcloud-admin.sh` (lee en oculto y pasa
  por stdin a `occ`, sin TTY anidado).
- **Jellyfin:** imagen oficial, direct play; administración desde su web.
- **Cwd de Podman:** todo comando como `services` se ejecuta **desde
  `/opt/services`** (`cd /opt/services && runuser -u services -- env
  HOME=/opt/services XDG_RUNTIME_DIR=/run/user/997 …`); otro cwd da
  `cannot chdir` y stdout no es escribible (`permission denied` en
  `/dev/stdout`).

### Cockpit (consola web, 9090)

`cockpit` + `cockpit-podman` en `https://atlas-hub:9090` (LAN y tailnet).
Certificado **autofirmado** (`CN=atlas-hub`, caduca dic-2027): Safari permite
continuar; el webview de Hub Panel (WKWebView) lo rechaza, por eso el botón
«Abrir Cockpit» abre **Safari** vía `tauri-plugin-opener`. Si algún día se
quiere ventana propia, la vía es un certificado de Tailscale (§12).

## 6. El relay

- **Un solo archivo** `server/atlas_relay.py` (~1300 líneas) +
  `server/system_monitor.py` (panel). Solo stdlib, Python ≥ 3.11
  (`python3.13`). Sin pip, sin venv: así para siempre.
- **HTTP:** `ThreadingHTTPServer` escuchando EXCLUSIVAMENTE la IP LAN
  (`RELAY_HOST` vacío = autodetectada) y la de Tailscale, puerto **8765**.
  HTTP/1.1 con keep-alive. CORS multi-origen
  (`RELAY_CORS_ORIGIN`, lista separada por comas; se devuelve solo el Origin
  solicitante si está en la lista).
- **Servicio:** systemd **de sistema** `atlas-relay.service` (fuente
  `deploy/server/atlas-relay.service`), `User=atlas`,
  `WorkingDirectory=/opt/atlas/repo`, `Restart=on-failure`, arranca con
  `multi-user.target`; endurecido con `ProtectSystem=strict`,
  `ProtectHome=true`, `PrivateTmp=true`, `ReadWritePaths=/opt/atlas/repo/temp`.
- **Estado en disco** (`temp/atlas-relay/`, git-ignored): `sessions.json`
  (escritura atómica), `telegram-offset.json` y `relay.log` (acotado a 300
  líneas). Se puede borrar sin drama (pierdes historial); no lo borres «para
  probar».
- **Arranque:** lee `/opt/atlas/repo/.env` (dueño `atlas`, 600, NUNCA en Git).
  Si no halla la IP LAN reintenta la autodetección hasta 60 s; si sigue en
  loopback, sale con exit 2 para que `Restart=on-failure` reintente (nunca se
  queda escuchando en `127.0.0.1`). Al reiniciar, las sesiones no terminales
  vuelven a `idle`; el `seq` sobrevive en disco.

**Ciclo de una petición:**

1. `POST /v1/send` (app) o mensaje Telegram aprobado → `Brain.receive()`:
   mensaje + evento `thinking` + **cola FIFO** (1 worker).
2. Worker → Groq (máx. 4 turnos, `openai/gpt-oss-120b` por defecto, claves
   rotativas `GROQ_API_KEY_1..3`).
3. Herramientas: `calendar_read` (automática) y `calendar_write`
   (**confirmación**: tarjeta en la app y/o DM Telegram; texto `sí/adelante` o
   `no`; timeout 300 s). Calendarios editables: `CALDAV_EDIT_CALENDARS`
   (defecto Exámenes,Eventos,Horario,Deberes).
4. Éxito → `completed` + respuesta por Telegram si vino de ahí; error →
   `failed` (`timeout`, `http_*`, `max_turns`…). Cancelable con `POST
   /v1/cancel` o en masa desde el panel.

Estados: `idle, thinking, awaiting_confirmation, executing, responding,
completed, failed, cancelled` (terminales: `idle/completed/failed/cancelled`).

**Telegram:** long-poll `getUpdates` (timeout 25 s), offset persistido, backoff
5→60 s. Solo acepta `TELEGRAM_ALLOWED_USER_IDS`; con token y sin ese set el
relay **no arranca** (exit 2). `getMe` aparece en `/v1/health` →
`bot.username`. Las confirmaciones se pueden resolver por texto.

**Endpoints** (detalle en [`relay-api.md`](./relay-api.md)):

| Ruta | Qué |
|---|---|
| `GET /v1/health` | ok, telegram, llm, calendar, events, queue, inflight, bot |
| `GET /v1/session` | sesión activa + mensajes + cursor |
| `GET /v1/events?since=N` | feed de eventos (poll 4 s en el front) |
| `GET /v1/logs?tail=N` | log del relay (máx. 300 líneas) |
| `GET /v1/stats` | tokens por modelo |
| `POST /v1/send /v1/cancel /v1/confirm` | ciclo de conversación |
| `GET /v1/calendar/*` | solo lectura (CalDAV de Nextcloud) |
| `GET /v1/system` | snapshot del host (unidades, puertos, recursos) |
| `POST /v1/system/asks` + `/v1/system/resolve` | acciones systemd con doble confirmación (op-id TTL 30 s) |
| `POST /v1/system/cancel_all` | cancela peticiones en vuelo (doble confirmación) |
| `GET /v1/hefesto/status` | `{configured, deviceId, suspended, generation}` |
| `POST /v1/hefesto/suspend` | suspende Hefesto por control remoto (reactivar es manual en la app) |
| `GET /panel/*` | estático del panel (build Vite de `apps/panel/dist`) |

## 7. Panel web (`/panel/`)

Panel en `/panel/` (build Vite de `apps/panel/dist`, versionado con
`git add -f` pese a `dist/` en `.gitignore`):

- **Portada:** barra `HUB PANEL` con hora del snapshot y píldora de estado (LED
  + servicios activos / en vuelo / cola), más KPIs (servicios, peticiones,
  memoria, disco, uptime, energía).
- **Tarjetas:** Servicios systemd (ssh, avahi, ufw, atlas-relay, tailscaled,
  cockpit), Puertos, **Servicios** (sondas HTTP de Nextcloud/Jellyfin +
  «Abrir Cockpit» → Safari), Relay Atlas (bot, LLM, calendario, tokens, cola),
  Peticiones y Log del relay.
- **Frescura:** sondea `/v1/system`, `/v1/health`, `/v1/session` y `/v1/logs`
  cada 2 s, `/v1/stats` cada 5 s; el snapshot del host se cachea 3 s. Sin
  emojis: texto + LED/icono.
- **Acciones systemd:** lista blanca `PANEL_ACTION_UNITS`, **vacía** por
  defecto en atlas-hub (faltan reglas polkit para `atlas`): ninguna acción de
  unidad prospera. El flujo de doble confirmación sigue intacto (botón →
  `asks` → tarjeta ámbar → `resolve allow_once/deny`).

## 8. Despliegue y operación desde el Mac

```bash
# relay: commit antes (el script NO commitea)
./scripts/deploy-relay.sh          # push → pull en atlas-hub → restart → health
./scripts/deploy-relay.sh --push   # sube lo ya commiteado sin exigir árbol limpio
./scripts/deploy-relay.sh --pull   # solo baja cambios y reinicia

# panel: build estático antes de desplegar (lo sirve el relay)
cd apps/panel && npm install && npm run build   # luego git add -f apps/panel/dist

# logs en el hub
ssh atlas-hub "journalctl -u atlas-relay -f"
ssh atlas-hub "tail -f /opt/atlas/repo/temp/atlas-relay/relay.log"
```

- **`.env` real** solo en `/opt/atlas/repo/.env` (600, `atlas`). Para
  actualizarlo:

```bash
scp .env adria_admin@atlas-hub:/tmp/atlas-env
ssh -t adria_admin@atlas-hub 'sudo install -m 600 -o atlas -g atlas /tmp/atlas-env /opt/atlas/repo/.env && rm -f /tmp/atlas-env'
```

- **Instaladores (una vez cada uno, piden la contraseña de `adria_admin`):**
  `./scripts/deploy-healthcheck.sh` (watchdog) y `./scripts/deploy-backup.sh`
  (backup semanal). Ambos necesitan que el relay ya esté desplegado (el script
  vive en `/opt/atlas/repo`).
- **Servicios:** pull en el hub + `sudo bash
  /opt/atlas/repo/deploy/atlas-hub/setup-services.sh` (§5).
- Variables del deploy: `ATLAS_DEPLOY_HOST` (defecto `atlas-hub`) y
  `ATLAS_RELAY_URL` (defecto `http://atlas-hub.local:8765/v1/health`).

## 9. Estabilidad y copias

**Checklist antes/después de cada deploy:**

1. `git status --short` vacío (el deploy aborta si hay cambios).
2. `curl http://atlas-hub.local:8765/v1/health` → `ok:true`, telegram/llm/calendar ok.
3. Después: repetir health + `GET /v1/session` (cursor no retrocede) + una
   petición local de prueba.
4. Si tocaste `PANEL_*`: re-probar el panel en LAN (las acciones siguen sin
   habilitarse sin polkit).

### Watchdog del relay (`atlas-health.timer`)

- Instalación: `./scripts/deploy-healthcheck.sh` → `atlas-health.service`
  (oneshot, root) + timer (90 s tras el arranque, luego cada 60 s).
- Comprueba `/v1/health` en la IP LAN (nunca loopback), timeout 3 s, y
  reinicia `atlas-relay` tras **2 fallos seguidos**. Contador en
  `/var/lib/atlas-health/failures`, a cero tras cada intento.
- Ver: `systemctl list-timers atlas-health.timer` y
  `journalctl -u atlas-health -n 30`.
- **Antes de parar el relay a mano:** `sudo systemctl stop atlas-health.timer`;
  después, `sudo systemctl start atlas-health.timer` (si no, lo levanta en 1-2
  min).
- Prueba en seco:
  `ATLAS_HEALTH_URL=http://127.0.0.1:9 ATLAS_HEALTH_DRY_RUN=1 ATLAS_HEALTH_STATE_DIR=/tmp/atlas-health python3 deploy/server/atlas-healthcheck.py`
  dos veces → `1/2` y `DRY RUN: reiniciaría atlas-relay`.
- Verificado el 2026-10-25: parado a las 23:24, reiniciado a las 23:26:01.

### Backup semanal (`atlas-backup.timer`)

- Instalación: `./scripts/deploy-backup.sh` → `atlas-backup.service` (oneshot,
  root) + timer (domingos 03:00, `Persistent=true`, retardo aleatorio ≤ 1 h).
- Destino: microSD o USB **exFAT** etiqueta `ATLASBACKUP` (montaje por
  etiqueta; rotación manual entre 2 unidades, p. ej. tarjeta + USB). **Solo una
  unidad con esa etiqueta conectada a la vez**: con dos, el servicio falla en
  vez de elegir al azar. En `<unidad>/<fecha>/`: volúmenes
  `mariadb-data`, `nextcloud-config`, `nextcloud-data`, `jellyfin-config`
  (export de Podman comprimido), `secrets`, `media`, `.env` del relay y
  `SHA256SUMS`. Conserva los 2 backups más recientes.
- **Sin tarjeta no es error** (sale 0; el timer no falla). Nunca escribe en el
  disco interno (aborta si la etiqueta resuelve a `/dev/sda*`).
- Podman corre como `services` desde `/opt/services` y exporta a un temporal
  suyo (`/opt/services/.atlas-backup-*.tar`) antes de comprimir: no puede
  escribir `/dev/stdout` y un export vacío haría un backup hueco. Si queda
  vacío, el servicio **falla** en vez de guardar basura. Hace falta espacio
  libre en `/opt` para el mayor volumen.
- Ver: `systemctl list-timers atlas-backup.timer` y
  `journalctl -u atlas-backup -n 50`. Prueba:
  `sudo systemctl start atlas-backup.service` con la tarjeta puesta.
- Restaurar: `podman volume import <volumen> < <volumen>.tar.gz` como
  `services`; extraer `secrets` y `.env` con sus permisos.
- Verificado el 2026-11-20: nextcloud-config 157 MB, nextcloud-data 68 MB,
  mariadb-data 22 MB, jellyfin-config 34 KB; `sha256sum -c` y `gzip -t` OK.

### Prohibiciones explícitas

- NO martillear iCloud CalDAV: cada prueba real requiere permiso explícito
  (throttle 403 de Apple ante ráfagas).
- NO abrir puertos a internet, NO webhooks de Telegram, NO auth por Telegram
  con permisos ampliados. Perímetro: LAN + tailnet.
- NO editar `temp/atlas-relay/*` «mientras corre».
- NO introducir dependencias en el relay (solo stdlib) ni bindings binarios.
- NO reiniciar `atlas-relay` fuera del flujo del deploy sin avisar.
- NO escribir el backup en `/dev/sda*` ni en APFS (Linux no escribe APFS).

## 10. Troubleshooting

| Síntoma | Causa probable | Acción |
|---|---|---|
| `403 same_network_only` | cliente fuera de la /24 y la tailnet | conectar a la LAN/VPN correcta |
| Telegram `401/404` en getMe | token inválido | el relay muere con exit 2; arreglar `.env` y redeploy |
| `502 calendar_upstream` / 403 iCloud | throttle de Apple o credencial | esperar 1-2 min; NO reintentar en bucle |
| relay en bucle de reinicio | error de arranque/config | `journalctl -u atlas-relay -n 50` |
| deploy falla en `sudo -u atlas git …` | falta el sudoers mínimo | instalar `deploy/server/atlas-deploy.sudoers` (§11) |
| deploy falla en `sudo systemctl restart` | unidad no instalada/rota | revisar `/etc/systemd/system/atlas-relay.service` (§11) |
| health no resuelve `atlas-hub.local` | avahi caído u otra subred | revisar avahi o usar `ATLAS_RELAY_URL` con la IP del momento |
| sin respuesta en `127.0.0.1:8765` en el hub | correcto: no escucha en loopback | consultar por la IP LAN o `atlas-hub.local` |
| relay «activo» pero panel/frontal no conectan | arrancó sin red y quedó en loopback | `ss -ltn \| grep 8765`; reiniciar con red |
| panel 404 en `/panel/` | falta `apps/panel/dist` o no se publicó | `cd apps/panel && npm run build` + `git add -f apps/panel/dist` |
| tarjeta Servicios sin botones de acción | `PANEL_ACTION_UNITS` vacía (sin polkit para `atlas`) | consciente: instalar polkit y rellenar `.env` |
| Nextcloud/Jellyfin dan **502** | contenedor caído | `sudo -u services env HOME=/opt/services XDG_RUNTIME_DIR=/run/user/997 podman ps` y `… systemctl --user status nextcloud jellyfin` |
| contenedores no arrancan tras reiniciar el hub | faltaba `[Install] WantedBy=default.target` en el quadlet | añadirlo, re-instalar unidades y `daemon-reload` (§5) |
| `cannot chdir`/`/dev/stdout: permission denied` con podman | cwd inaccesible o salida a stdout como `services` | `cd /opt/services` y exportar a fichero (§5, §9) |
| backup con tarballs de 20 B | export vacío enmascarado por `| gzip` | usar la versión actual del script y mirar tamaños, no solo el exit 0 |
| microSD en solo-lectura (`ro`=1) | WP leído solo al insertar | reinsertarla de verdad (sacar y meter hasta el clic) |
| `sudo` rechaza la contraseña | valor tecleado/pegado distinto | sin root ni consola: recuperación física por GRUB (`rw init=/bin/bash` y `passwd`) |
| primer handshake lento (~0,2 s) | `.local` prueba IPv6 link-local antes que IPv4 | normal; con keep-alive los endpoints cálidos van a ~10 ms |

## 11. Instalación desde cero (resumen)

El sudo de la instalación es interactivo (contraseña); el sudoers sin
contraseña solo sirve para el deploy automatizado posterior.

1. **Base:** hostname `atlas-hub`; `adria_admin` con la clave pública del Mac
   en `authorized_keys`; usuario `atlas` (home `/opt/atlas`, sin login) dueño
   de `/opt/atlas/repo` (repo público clonado por HTTPS); `.env` instalado
   con permisos 600 (`atlas`).
2. **Relay:**

```bash
sudo visudo -cf /opt/atlas/repo/deploy/server/atlas-deploy.sudoers
sudo install -m 644 -o root -g root /opt/atlas/repo/deploy/server/atlas-relay.service /etc/systemd/system/
sudo install -m 440 -o root -g root /opt/atlas/repo/deploy/server/atlas-deploy.sudoers /etc/sudoers.d/atlas-deploy
sudo visudo -cf /etc/sudoers.d/atlas-deploy
sudo install -d -o atlas -g atlas /opt/atlas/repo/temp
sudo systemctl daemon-reload && sudo systemctl enable --now atlas-relay
curl http://atlas-hub.local:8765/v1/health   # NO vale 127.0.0.1
```

3. **Perímetro:** `sudo bash /opt/atlas/repo/deploy/atlas-hub/ufw.sh` (idempotente).
4. **Tailscale:** `sudo bash /opt/atlas/repo/deploy/atlas-hub/install.sh` y
   `sudo tailscale up` (login por navegador). Mantener parcheado.
5. **Cockpit:** `sudo apt-get install -y cockpit cockpit-podman` (socket en
   9090; reglas ufw ya lo cubren).
6. **Servicios:** `sudo bash /opt/atlas/repo/deploy/atlas-hub/setup-services.sh`
   (quadlets, secretos, imágenes, trusted_domains, comprobación de MariaDB).
   Completar el asistente web de Nextcloud con los datos de
   `/opt/services/secrets/mariadb.env`.
7. **CalDAV:** `sudo bash /opt/atlas/repo/deploy/atlas-hub/set-nextcloud-env.sh`
   y redeploy del relay.
8. **Extras:** `./scripts/deploy-healthcheck.sh` y `./scripts/deploy-backup.sh`
   desde el Mac (tras el primer deploy). Formatear la unidad como exFAT con
   etiqueta `ATLASBACKUP` (11 caracteres máx., solo en una unidad que no tenga
   datos personales): `sudo wipefs -a /dev/sdX && sudo parted -s /dev/sdX
   mklabel gpt mkpart primary 0% 100% && sudo mkfs.exfat -n ATLASBACKUP
   /dev/sdX1`.
9. **Verificación:** desde el Mac `./scripts/deploy-relay.sh --pull`; desde el
   iPhone fuera de casa, llegar por Tailscale a Nextcloud/Jellyfin.

## 12. Estado y pendientes

- **Implementado:** relay en systemd de sistema; Nextcloud + MariaDB y Jellyfin
  en Podman rootless con arranque al boot; Cockpit; Tailscale; `ufw`;
  watchdog del relay; backup semanal a microSD; contraseñas unificadas en el
  hub; 76 eventos migrados de iCloud a Nextcloud.
- **Pendientes declarados:** prueba real desde el iPhone fuera de casa
  (Tailscale); rotar la segunda tarjeta de backup; `tailscaled` parcheado y
  revisado en cada mantenimiento.
- **Mejoras futuras:** HTTPS con certificado de Tailscale (`tailscale cert`)
  para servicios y, con él, ventana propia de Cockpit en Hub Panel; media en
  disco externo cuando la librería crezca; contraseña de aplicación real para
  CalDAV en vez de la del admin.
