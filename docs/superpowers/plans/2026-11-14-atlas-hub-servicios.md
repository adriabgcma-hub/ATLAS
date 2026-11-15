# Atlas Hub (servidor de servicios + acceso remoto) — Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convertir `atlas-hub` en un servidor de servicios (Nextcloud + Jellyfin) accesible desde fuera de casa (Tailscale) con Cockpit como consola y Hub Panel como resumen.

**Architecture:** Piezas nuevas aisladas en `atlas-hub` (Tailscale, Cockpit, Podman rootless + Quadlet); el relay y Hefesto solo amplían su perímetro de red. Configs del servidor versionados en `deploy/atlas-hub/`; cambios de relay en `server/` desplegados con `./scripts/deploy-relay.sh`.

**Tech Stack:** Debian 13 · Tailscale (cliente tailscaled) · Cockpit + cockpit-podman · Podman rootless 5.4 + Quadlet · MariaDB + linuxserver/nextcloud + jellyfin · Python stdlib (relay) · React/Tauri (panel).

**Spec:** [docs/atlas-hub-plan.md](../atlas-hub-plan.md) — el plan argumenta desde la spec; léela antes de ejecutar.

## Global Constraints

- Sin secretos en el repo: `.env`, contraseña de app de Nextcloud, claves *pre-auth* y passphrases viven fuera de Git (`.env` del relay en `/opt/atlas/repo/.env` modo 600; secretos de servicios en `/opt/services/secrets/` modo 600).
- Commits **solo con permiso explícito del usuario** (AGENTS.md); al concederse, publicar al remoto. Nada de push/rebase sin permiso.
- iCloud CalDAV: **una sola operación por prueba, con permiso explícito y pausa** entre intentos (Apple throttle 403).
- Relay: **solo stdlib**, sin dependencias nuevas. Tras tocar `server/`, commitear y `./scripts/deploy-relay.sh`.
- Cambios en `atlas-hub` (instalar paquetes, tocar systemd/Tailscale/Cockpit) requieren **permiso explícito por operación**.
- Tailscale: mantener el cliente `tailscaled` **parcheado y revisado** (AGENTS.md). No se abre ningún puerto a internet.
- Ciclo de vida: nada de autoarranque ni procesos huérfanos; matar por PID propio y verificar `ps`/`pgrep` al terminar.
- Configs del servidor versionados en `deploy/` (patrón existente `deploy/server/atlas-relay.service`).
- Verificación del relay: `curl http://atlas-hub.local:8765/v1/health` tras cada deploy.

---

### Task 1: Crear cuenta Tailscale (manual, usuario)

**Files:** ninguno.

**Interfaces:**
- Produce: una cuenta Tailscale (Google/GitHub/etc.) que usarán atlas-hub, el Mac y el iPhone.

- [ ] **Step 1: Crear la cuenta**

En https://login.tailscale.com crea una cuenta (Google, GitHub, etc.). No hace falta DuckDNS ni tocar el router: Tailscale atraviesa el NAT sin puertos abiertos.

- [ ] **Step 2: Verificar acceso al panel**

Entra en https://login.tailscale.com/admin y confirma que ves tu cuenta (aún sin máquinas).

---

### Task 2: Esqueleto versionado `deploy/atlas-hub/` (repo)

**Files:**
- Create: `deploy/atlas-hub/install.sh`
- Create: `deploy/atlas-hub/ufw.sh`

**Interfaces:**
- Produce: `deploy/atlas-hub/install.sh` (instala el cliente tailscale desde el repo oficial), `deploy/atlas-hub/ufw.sh` (reglas ufw idempotentes, nada abierto a internet).

- [ ] **Step 1: Escribir `deploy/atlas-hub/install.sh`**

```bash
#!/usr/bin/env bash
# Instala el cliente tailscale desde el repo oficial (Debian) y lo arranca.
set -euo pipefail
sudo apt-get update
sudo apt-get install -y curl
curl -fsSL https://tailscale.com/install.sh | sudo sh
sudo systemctl enable --now tailscaled
```

- [ ] **Step 2: Escribir `deploy/atlas-hub/ufw.sh`**

```bash
#!/usr/bin/env bash
# Reglas de perímetro. Idempotente. Nada abierto a internet: solo LAN + tailnet.
set -euo pipefail
TAILNET="100.64.0.0/10"
sudo ufw allow from 192.168.100.0/24 to any port 9090 proto tcp comment 'cockpit LAN'
sudo ufw allow from "${TAILNET}" to any port 9090 proto tcp comment 'cockpit tailnet'
sudo ufw allow from "${TAILNET}" to any port 8765 proto tcp comment 'relay tailnet'
sudo ufw allow from "${TAILNET}" to any port 8767 proto tcp comment 'hefesto tailnet'
sudo ufw allow from "${TAILNET}" to any port 8080 proto tcp comment 'nextcloud tailnet'
sudo ufw allow from "${TAILNET}" to any port 8096 proto tcp comment 'jellyfin tailnet'
sudo ufw status numbered
```

- [ ] **Step 3: Hacer ejecutables y commitear**

```bash
chmod +x deploy/atlas-hub/install.sh deploy/atlas-hub/ufw.sh
git add deploy/atlas-hub
git commit -m "feat(atlas-hub): instalación de tailscale y reglas ufw versionadas"
```

(Commit con permiso explícito del usuario; publicar al remoto al concederse.)

---

### Task 3: Instalar tailscaled en atlas-hub y unirlo

**Files:** ninguno nuevo (usa los de la Task 2).

**Interfaces:**
- Consume: `deploy/atlas-hub/install.sh`, `deploy/atlas-hub/ufw.sh`, cuenta Tailscale (Task 1).
- Produce: `tailscaled` activo en atlas-hub, unido a la tailnet con IP 100.64.x.y.

- [ ] **Step 1: Copiar el repo actualizado a atlas-hub**

Primero commitear y hacer push (permiso del usuario), luego:

```bash
ssh adria_admin@atlas-hub "sudo -u atlas git -C /opt/atlas/repo pull"
```

- [ ] **Step 2: Ejecutar instalación + ufw**

```bash
ssh adria_admin@atlas-hub "cd /opt/atlas/repo && sudo bash deploy/atlas-hub/install.sh"
ssh adria_admin@atlas-hub "cd /opt/atlas/repo && sudo bash deploy/atlas-hub/ufw.sh"
```

- [ ] **Step 3: Unir atlas-hub a la tailnet (login manual del usuario)**

```bash
ssh -t adria_admin@atlas-hub "sudo tailscale up"
```

Imprime una URL de login. El usuario la abre en el navegador e inicia sesión con su cuenta Tailscale (Task 1). Al aceptar, atlas-hub queda unido.

- [ ] **Step 4: Verificar**

```bash
ssh adria_admin@atlas-hub "tailscale status; tailscale ip -4"
```

Expected: `tailscale status` lista atlas-hub con una IP `100.64.x.y`; `tailscale ip -4` la imprime. Anota esa IP (se usa en la Task 5).

---

### Task 4: Conectar Mac + iPhone y verificar desde fuera

**Files:** ninguno.

**Interfaces:**
- Consume: atlas-hub unido a la tailnet (Task 3), cuenta Tailscale (Task 1).

- [ ] **Step 1: Instalar Tailscale en el Mac y unirlo**

Descarga Tailscale para macOS (App Store o tailscale.com), instálalo e inicia sesión con la misma cuenta. Verifica:

```bash
tailscale status   # debe listar el Mac y atlas-hub con IPs 100.64.x.y
```

- [ ] **Step 2: Unir el iPhone (datos móviles, fuera de la WiFi de casa)**

App Tailscale en el iPhone → iniciar sesión con la misma cuenta.

- [ ] **Step 3: Verificar desde fuera de casa**

Con el iPhone en datos móviles (o el Mac fuera de casa):

```bash
tailscale ping atlas-hub                        # debe responder (directa o vía DERP)
curl -s http://atlas-hub:8765/v1/health         # nombre MagicDNS (requiere Task 5 completada)
```

Expected: `tailscale ping atlas-hub` OK y, tras la Task 5, `curl http://atlas-hub:8765/v1/health` devuelve `ok:true`.

---

### Task 5: El relay acepta la red Tailscale (código + deploy)

**Files:**
- Modify: `server/atlas_relay.py` (read_env, `client_allowed`, `make_handler`, `main`, `hefesto_listener`)
- Test: `server/test_relay_perimeter.py` (nuevo)

**Interfaces:**
- Consume: `RELAY_EXTRA_HOSTS` y `RELAY_ALLOW_NETWORKS` (nuevas variables, opcionales).
- Produce: `client_allowed(client_ip: str, networks: list) -> bool` (función de módulo); el relay escucha en la IP LAN **y** en las de `RELAY_EXTRA_HOSTS`; Hefesto escucha en la LAN y en las extra.

- [ ] **Step 1: Escribir el test que falla**

```python
# server/test_relay_perimeter.py
import ipaddress
import unittest

from atlas_relay import client_allowed


class TestClientAllowed(unittest.TestCase):
    def test_cliente_lan_permitido(self):
        nets = [ipaddress.ip_network("192.168.100.0/24")]
        self.assertTrue(client_allowed("192.168.100.26", nets))

    def test_cliente_fuera_rechazado(self):
        nets = [ipaddress.ip_network("192.168.100.0/24")]
        self.assertFalse(client_allowed("8.8.8.8", nets))

    def test_cliente_tailnet_permitido_con_red_extra(self):
        nets = [ipaddress.ip_network("192.168.100.0/24"),
                ipaddress.ip_network("100.64.0.0/10")]
        self.assertTrue(client_allowed("100.64.0.5", nets))

    def test_ip_invalida_rechazada(self):
        nets = [ipaddress.ip_network("192.168.100.0/24")]
        self.assertFalse(client_allowed("no-es-ip", nets))


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Ejecutar el test y verlo fallar**

Run: `cd server && python3 -m unittest test_relay_perimeter -v`
Expected: FAIL — `ImportError: cannot import name 'client_allowed'`.

- [ ] **Step 3: Implementar `client_allowed` y el perímetro por lista de redes**

En `server/atlas_relay.py`, junto a `lan_ip()` (línea ~721), añadir:

```python
def client_allowed(client_ip: str, networks: list) -> bool:
    """True si client_ip es IPv4 y cae dentro de alguna red permitida."""
    try:
        client = ipaddress.ip_address(client_ip)
    except ValueError:
        return False
    return isinstance(client, ipaddress.IPv4Address) and any(
        client in n for n in networks)
```

En `read_env()` (línea ~714), añadir al dict devuelto:

```python
        "extra_hosts": [h for h in
                        os.environ.get("RELAY_EXTRA_HOSTS", "").split(",")
                        if h.strip()],
        "allow_networks": [n for n in
                           os.environ.get("RELAY_ALLOW_NETWORKS", "").split(",")
                           if n.strip()],
```

En `make_handler` (línea 2442), cambiar el parámetro `network` a `networks`, y en `_lan_only` (línea ~2472):

```python
        def _lan_only(self) -> bool:
            return client_allowed(self.client_address[0], networks)
```

- [ ] **Step 4: Multi-bind en `main()`**

Sustituir el bloque de bind único (líneas ~2864-2893) por:

```python
    hosts = [host] + cfg["extra_hosts"]
    for h in hosts:
        try:
            ipaddress.ip_address(h)
        except ValueError:
            fail_config(f"RELAY_HOST/RELAY_EXTRA_HOSTS no es una IP válida: {h!r}")
    networks = [ipaddress.ip_network(h + "/24", strict=False) for h in hosts]
    for n in cfg["allow_networks"]:
        networks.append(ipaddress.ip_network(n, strict=False))
    ...
    handler = make_handler(brain, networks, status, sysmon)
    servers = [ThreadingHTTPServer((h, cfg["port"]), handler) for h in hosts]
    with ExitStack() as stack:
        for srv in servers:
            stack.callback(srv.server_close)
            threading.Thread(target=srv.serve_forever, daemon=True,
                             name="relay-http").start()
        with hefesto_listener(hosts, cfg["data_dir"]) as hefesto_ctx:
            ...
            log(f"escuchando en {', '.join(hosts)}:{cfg['port']} "
                f"(redes {', '.join(str(n) for n in networks)}).")
```

- [ ] **Step 5: `hefesto_listener` acepta una lista de hosts**

Cambiar la firma `def hefesto_listener(host: str, data_dir: str)` a `def hefesto_listener(hosts: list[str], data_dir: str)` y, donde se crea el server (`hefesto.create_server((host, port), ...)`), crear uno por host:

```python
    servers = [hefesto.create_server(
        (h, port), journal, peers,
        certfile=os.environ["HEFESTO_TLS_CERT"],
        keyfile=os.environ["HEFESTO_TLS_KEY"],
        cafile=os.environ["HEFESTO_TLS_CA"],
    ) for h in hosts]
    for srv in servers:
        resources.callback(srv.server_close)
    # en serve(): serve_forever() por cada server en su hilo; stop(): shutdown de todos
```

- [ ] **Step 6: Ejecutar tests y verlos pasar**

Run: `cd server && python3 -m unittest test_relay_perimeter -v && python3 -m unittest discover -q`
Expected: PASS en todos (los tests existentes de hefesto/relay siguen verdes).

- [ ] **Step 7: Commitear y desplegar (permiso del usuario)**

```bash
git add server/atlas_relay.py server/test_relay_perimeter.py
git commit -m "feat(relay): perímetro multi-red (LAN + Tailscale) y multi-bind"
./scripts/deploy-relay.sh
```

- [ ] **Step 8: Configurar `.env` del relay en atlas-hub**

Añadir al `/opt/atlas/repo/.env` (dueño `atlas`, 600; con el comando de AGENTS.md para pasarlo):

```
RELAY_EXTRA_HOSTS=100.64.0.1
RELAY_ALLOW_NETWORKS=100.64.0.0/10
```

(La IP tailnet de atlas-hub se confirma con `tailscale ip -4` en atlas-hub.) Reiniciar: `ssh adria_admin@atlas-hub "sudo systemctl restart atlas-relay"`.

- [ ] **Step 9: Verificar**

```bash
curl http://atlas-hub.local:8765/v1/health        # ok:true (LAN)
curl http://atlas-hub:8765/v1/health              # ok:true (tailnet, desde un dispositivo unido)
```

---

### Task 6: Instalar Cockpit + cockpit-podman

**Files:** ninguno nuevo.

**Interfaces:**
- Produce: `cockpit.service` activo escuchando en 9090 (LAN + tailnet).

- [ ] **Step 1: Instalar**

```bash
ssh adria_admin@atlas-hub "sudo apt-get update && sudo apt-get install -y cockpit cockpit-podman"
```

- [ ] **Step 2: Limitar el bind a la LAN + tailnet**

Cockpit escucha en 9090 vía `cockpit-ws`. Para no exponerlo a internet, verificar que `ufw.sh` ya aplicó las reglas 9090 (Task 3) y que no hay regla 9090 abierta a `Anywhere`:

```bash
ssh adria_admin@atlas-hub "sudo ufw status numbered | grep 9090"
```

- [ ] **Step 3: Verificar**

```bash
curl -k https://atlas-hub.local:9090   # página de login de Cockpit (LAN)
curl -k https://atlas-hub:9090         # desde tailnet
```

Expected: HTML de Cockpit en ambos; login con `adria_admin`.

---

### Task 7: Podman rootless + usuario `services` + linger

**Files:** ninguno nuevo.

**Interfaces:**
- Produce: usuario de sistema `services` (sin login, home `/opt/services`) con podman rootless operativo y `linger` activo.

- [ ] **Step 1: Crear usuario y directorios**

```bash
ssh adria_admin@atlas-hub "sudo useradd --system --home-dir /opt/services --shell /usr/sbin/nologin services && sudo install -d -o services -g services /opt/services/{secrets,nextcloud,mariadb,jellyfin,media}"
```

- [ ] **Step 2: Instalar podman y activar linger**

```bash
ssh adria_admin@atlas-hub "sudo apt-get install -y podman && sudo loginctl enable-linger services"
```

- [ ] **Step 3: Verificar rootless**

```bash
ssh adria_admin@atlas-hub "sudo machinectl shell services@ /bin/sh -c 'podman info --format \"{{.Host.Security.Rootless}}\" && podman run --rm docker.io/library/hello-world:latest'"
```

Expected: `true` (rootless) y salida de hello-world.

---

### Task 8: Quadlet — MariaDB

**Files:**
- Create: `deploy/atlas-hub/quadlet/mariadb.container`
- Create: `deploy/atlas-hub/quadlet/network/atlas-services.network`

**Interfaces:**
- Produce: unidad de usuario `mariadb.service` (podman) en la red `atlas-services`, con volumen `mariadb-data`.

- [ ] **Step 1: Escribir la red**

```ini
# deploy/atlas-hub/quadlet/network/atlas-services.network
[Network]
Subnet=10.89.0.0/24
```

- [ ] **Step 2: Escribir el contenedor**

```ini
# deploy/atlas-hub/quadlet/mariadb.container
[Unit]
Description=MariaDB (Nextcloud)

[Container]
Image=docker.io/library/mariadb:11
EnvironmentFile=/opt/services/secrets/mariadb.env
Volume=mariadb-data:/var/lib/mysql:Z
Network=atlas-services.network
ContainerName=mariadb
```

- [ ] **Step 3: Crear el fichero de secretos (fuera del repo)**

```bash
ssh adria_admin@atlas-hub "printf 'MARIADB_ROOT_PASSWORD=<generar>\nMARIADB_DATABASE=nextcloud\nMARIADB_USER=nextcloud\nMARIADB_PASSWORD=<generar>\n' | sudo install -m 600 -o services -g services /dev/stdin /opt/services/secrets/mariadb.env"
```

- [ ] **Step 4: Instalar y arrancar la unidad**

```bash
ssh adria_admin@atlas-hub "sudo cp deploy/atlas-hub/quadlet/network/atlas-services.network deploy/atlas-hub/quadlet/mariadb.container /opt/services/.config/containers/systemd/ 2>/dev/null || true"
ssh adria_admin@atlas-hub "sudo -u services systemctl --user daemon-reload && sudo -u services systemctl --user start mariadb.service"
```

(Crear el directorio `/opt/services/.config/containers/systemd` si no existe.)

- [ ] **Step 5: Verificar**

```bash
ssh adria_admin@atlas-hub "sudo -u services systemctl --user is-active mariadb.service && sudo -u services podman exec mariadb mariadb -unextcloud -p\$(sudo cat /opt/services/secrets/mariadb.env | grep MARIADB_PASSWORD | cut -d= -f2) -e 'SELECT 1;'"
```

Expected: `active` y `1` (conexión OK).

---

### Task 9: Quadlet — Nextcloud

**Files:**
- Create: `deploy/atlas-hub/quadlet/nextcloud.container`

**Interfaces:**
- Consume: `mariadb.service` (Task 8).
- Produce: `nextcloud.service` publicando 8080, conectada a MariaDB, volúmenes `nextcloud-config` y `nextcloud-data`.

- [ ] **Step 1: Escribir el contenedor**

```ini
# deploy/atlas-hub/quadlet/nextcloud.container
[Unit]
Description=Nextcloud
Requires=mariadb.service
After=mariadb.service

[Container]
Image=docker.io/linuxserver/nextcloud:latest
EnvironmentFile=/opt/services/secrets/nextcloud.env
Volume=nextcloud-config:/config:Z
Volume=nextcloud-data:/data:Z
PublishPort=8080:80
Network=atlas-services.network
ContainerName=nextcloud
```

- [ ] **Step 2: Crear secretos (fuera del repo)**

```bash
ssh adria_admin@atlas-hub "printf 'PUID=1000\nPGID=1000\nTZ=Europe/Madrid\n' | sudo install -m 600 -o services -g services /dev/stdin /opt/services/secrets/nextcloud.env"
```

- [ ] **Step 3: Instalar y arrancar**

```bash
ssh adria_admin@atlas-hub "sudo cp deploy/atlas-hub/quadlet/nextcloud.container /opt/services/.config/containers/systemd/"
ssh adria_admin@atlas-hub "sudo -u services systemctl --user daemon-reload && sudo -u services systemctl --user start nextcloud.service"
```

- [ ] **Step 4: Verificar (crear admin y subir archivo)**

Abrir `http://atlas-hub:8080` (tailnet) o `http://atlas-hub.local:8080` (LAN). Si linuxserver/nextcloud redirige a HTTPS con certificado autofirmado, aceptar el aviso una vez (o publicar también `8443:443`).

1. Crear el usuario admin.
2. Subir un archivo de prueba y descargarlo.

Expected: web accesible, admin creado, archivo subido/descargado.

- [ ] **Step 5: Commitear los Quadlet (permiso del usuario)**

```bash
git add deploy/atlas-hub/quadlet
git commit -m "feat(atlas-hub): quadlets de mariadb y nextcloud"
```

---

### Task 10: Quadlet — Jellyfin

**Files:**
- Create: `deploy/atlas-hub/quadlet/jellyfin.container`

**Interfaces:**
- Produce: `jellyfin.service` publicando 8096, volumen `jellyfin-config`/`jellyfin-cache`, media montada de `/opt/media`.

- [ ] **Step 1: Escribir el contenedor**

```ini
# deploy/atlas-hub/quadlet/jellyfin.container
[Unit]
Description=Jellyfin

[Container]
Image=docker.io/jellyfin/jellyfin:latest
Volume=jellyfin-config:/config:Z
Volume=jellyfin-cache:/cache:Z
Volume=/opt/media:/media:ro
PublishPort=8096:8096
Network=atlas-services.network
ContainerName=jellyfin
```

- [ ] **Step 2: Instalar y arrancar**

```bash
ssh adria_admin@atlas-hub "sudo cp deploy/atlas-hub/quadlet/jellyfin.container /opt/services/.config/containers/systemd/"
ssh adria_admin@atlas-hub "sudo -u services systemctl --user daemon-reload && sudo -u services systemctl --user start jellyfin.service"
```

- [ ] **Step 3: Verificar**

Abrir `http://atlas-hub:8096` (tailnet). Completar el asistente, añadir `/media`, reproducir un vídeo (direct play).

Expected: reproducción OK sin transcodificar.

- [ ] **Step 4: Commitear (permiso del usuario)**

```bash
git add deploy/atlas-hub/quadlet/jellyfin.container
git commit -m "feat(atlas-hub): quadlet de jellyfin (direct play)"
```

---

### Task 11: Migrar eventos de calendario de iCloud a Nextcloud

**Files:** ninguno.

**Interfaces:**
- Consume: credenciales iCloud (permiso explícito del usuario; **una sola operación**), Nextcloud activo (Task 9).

- [ ] **Step 1: Exportar los calendarios de iCloud (una petición, con permiso)**

En el Mac, en la app Calendario → Archivo → Exportar → Exportar… → guardar `.ics` por cada calendario a migrar. (O desde Ajustes de iCloud, exportar cada calendario.) Sin tocar el relay.

- [ ] **Step 2: Crear los mismos calendarios en Nextcloud**

En la web de Nextcloud → Calendario → Nuevo calendario, con los mismos nombres que usa el relay (`Exámenes`, `Celebraciones`, `Horario`, `Deberes`, o los que tenga `CALDAV_EDIT_CALENDARS`).

- [ ] **Step 3: Importar los `.ics`**

En cada calendario de Nextcloud → Ajustes → Importar calendario → subir el `.ics` correspondiente.

- [ ] **Step 4: Verificar antes de cambiar el relay**

Comprobar en la web de Nextcloud que los eventos aparecen (pasados y futuros). **No borrar nada de iCloud todavía.**

---

### Task 12: El CalDAV del relay apunta a Nextcloud

**Files:**
- Modify: `server/atlas_caldav.py` (BASE y WELL_KNOWN configurables)
- Modify: `server/atlas_relay.py` (leer `CALDAV_URL`)
- Test: `server/test_caldav_url.py` (nuevo)

**Interfaces:**
- Consume: `CALDAV_URL` (nueva variable; si vacía, iCloud como hoy).
- Produce: `caldav_base() -> str` (función de módulo).

- [ ] **Step 1: Escribir el test que falla**

```python
# server/test_caldav_url.py
import os
import unittest
from unittest import mock

import atlas_caldav


class TestCalDavUrl(unittest.TestCase):
    def test_por_defecto_icloud(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertEqual(atlas_caldav.caldav_base(),
                             "https://caldav.icloud.com")

    def test_con_variable_nextcloud(self):
        with mock.patch.dict(os.environ, {"CALDAV_URL": "https://atlas-hub:8080/remote.php/dav"}, clear=True):
            self.assertEqual(atlas_caldav.caldav_base(),
                             "https://atlas-hub:8080/remote.php/dav")


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Ejecutar y ver fallar**

Run: `cd server && python3 -m unittest test_caldav_url -v`
Expected: FAIL — `ImportError: cannot import name 'caldav_base'`.

- [ ] **Step 3: Implementar `caldav_base()`**

En `server/atlas_caldav.py`, sustituir las constantes por:

```python
def caldav_base() -> str:
    """Base CalDAV: CALDAV_URL si está definida; iCloud por defecto."""
    return os.environ.get("CALDAV_URL", "").strip() or "https://caldav.icloud.com"
```

Y sustituir `BASE = "https://caldav.icloud.com"` por `BASE = caldav_base()` y `WELL_KNOWN = caldav_base() + "/.well-known/caldav"` (manteniendo el resto del código que usa `BASE`). Importar `os` si no está ya.

- [ ] **Step 4: Verificar el test y el resto**

Run: `cd server && python3 -m unittest test_caldav_url -v && python3 -m unittest discover -q`
Expected: PASS en todos.

- [ ] **Step 5: Commitear y desplegar (permiso del usuario)**

```bash
git add server/atlas_caldav.py server/atlas_relay.py server/test_caldav_url.py
git commit -m "feat(relay): CALDAV_URL configurable para apuntar a Nextcloud"
./scripts/deploy-relay.sh
```

- [ ] **Step 6: Configurar `.env` del relay**

En `/opt/atlas/repo/.env`:

```
CALDAV_URL=https://atlas-hub:8080/remote.php/dav
CALDAV_USERNAME=<usuario nextcloud>
CALDAV_APP_PASSWORD=<contraseña de aplicación de Nextcloud>
CALDAV_EDIT_CALENDARS=Exámenes,Celebraciones,Horario,Deberes
```

Generar la contraseña de aplicación en Nextcloud → Ajustes → Seguridad → Contraseñas de aplicación. Reiniciar el relay.

- [ ] **Step 7: Verificar el ciclo real**

Preguntar a Atlas por el calendario (`calendar_read`) y pedir una escritura con confirmación. Expected: eventos de Nextcloud leídos; escritura aplicada en Nextcloud.

---

### Task 13: `system_monitor` expone estado de servicios

**Files:**
- Modify: `server/system_monitor.py` (probes HTTP + unidades tailscaled/cockpit)
- Test: `server/test_system_monitor_services.py` (nuevo)

**Interfaces:**
- Consume: `PANEL_SERVICE_PROBES` (nueva variable: `nombre=http://127.0.0.1:puerto` separado por comas), `PANEL_SYSTEM_UNITS` (existente).
- Produce: en el snapshot, campo `services: [{name, up, url}]`.

- [ ] **Step 1: Escribir el test que falla**

```python
# server/test_system_monitor_services.py
import os
import unittest
from unittest import mock

import system_monitor


class TestServiceProbes(unittest.TestCase):
    def test_parsea_probes(self):
        with mock.patch.dict(os.environ, {"PANEL_SERVICE_PROBES": "nextcloud=http://127.0.0.1:8080,jellyfin=http://127.0.0.1:8096"}, clear=True):
            mon = system_monitor.SystemMonitor()
            self.assertEqual(mon.service_probes, {
                "nextcloud": "http://127.0.0.1:8080",
                "jellyfin": "http://127.0.0.1:8096",
            })

    def test_sin_probes_vacio(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            mon = system_monitor.SystemMonitor()
            self.assertEqual(mon.service_probes, {})


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Ejecutar y ver fallar**

Run: `cd server && python3 -m unittest test_system_monitor_services -v`
Expected: FAIL — `AttributeError: 'SystemMonitor' object has no attribute 'service_probes'`.

- [ ] **Step 3: Implementar probes**

En `server/system_monitor.py`, en `__init__`:

```python
        self.service_probes = _parse_probes(
            os.environ.get("PANEL_SERVICE_PROBES", ""))
```

Y en el módulo:

```python
def _parse_probes(raw: str) -> dict[str, str]:
    out: dict[str, str] = {}
    for chunk in raw.split(","):
        chunk = chunk.strip()
        if not chunk or "=" not in chunk:
            continue
        name, url = chunk.split("=", 1)
        if name.strip() and url.strip().startswith("http"):
            out[name.strip()] = url.strip()
    return out
```

Método de sondeo (usa `urllib.request`, import al inicio del fichero):

```python
    def _services(self) -> list[dict]:
        import urllib.request
        items = []
        for name, url in self.service_probes.items():
            up = False
            try:
                with urllib.request.urlopen(url, timeout=2) as resp:
                    up = resp.status < 400
            except Exception:
                up = False
            items.append({"name": name, "url": url, "up": up})
        return items
```

Y en `snapshot()`, añadir `"services": self._services(),` al dict devuelto.

- [ ] **Step 4: Verificar tests y arranque**

Run: `cd server && python3 -m unittest test_system_monitor_services -v && python3 -m unittest discover -q`
Expected: PASS en todos.

- [ ] **Step 5: Commitear y desplegar (permiso del usuario)**

```bash
git add server/system_monitor.py server/test_system_monitor_services.py
git commit -m "feat(panel): sondeo HTTP de servicios en el snapshot"
./scripts/deploy-relay.sh
```

- [ ] **Step 6: Configurar `.env` del relay**

```
PANEL_SYSTEM_UNITS=ssh.service,avahi-daemon.service,ufw.service,atlas-relay.service,tailscaled.service,cockpit.service
PANEL_SERVICE_PROBES=nextcloud=http://127.0.0.1:8080,jellyfin=http://127.0.0.1:8096
```

Reiniciar el relay. Verificar `curl http://atlas-hub.local:8765/v1/system | python3 -m json.tool` muestra `services` y las unidades nuevas.

---

### Task 14: Hub Panel — tarjeta Servicios y botón Abrir Cockpit

**Files:**
- Modify: `apps/panel/src/api.ts` (tipo `Snapshot` con `services`)
- Modify: `apps/panel/src/components/Cards.tsx` (nueva `ServicesCard`)
- Modify: `apps/panel/src/App.tsx` (montar la tarjeta)

**Interfaces:**
- Consume: `snapshot.services` (Task 13), `snapshot.units` existente.
- Produce: componente `ServicesCard({ snap })` con puntos up/down y enlace a Cockpit.

- [ ] **Step 1: Ampliar el tipo en `api.ts`**

En `export interface Snapshot`, añadir tras `ports`:

```ts
  services: { name: string; url: string; up: boolean }[]
```

- [ ] **Step 2: Escribir la tarjeta en `Cards.tsx`**

```tsx
export function ServicesCard({ snap }: { snap: Snapshot }) {
  return (
    <Card title="Servicios" badge={`${snap.services.length} sondas`}>
      {snap.services.map(s => (
        <div className="row" key={s.name}>
          <span className={`dot ${s.up ? 'ok' : 'dim'}`}></span>
          <div className="grow">
            <div className="name">{s.name}</div>
            <div className="sub">{s.up ? 'activo' : 'sin respuesta'}</div>
          </div>
        </div>
      ))}
      <div style={{ marginTop: 8 }}>
        <a className="btn" href="https://atlas-hub:9090" target="_blank" rel="noreferrer">
          Abrir Cockpit
        </a>
      </div>
    </Card>
  )
}
```

- [ ] **Step 3: Montarla en `App.tsx`**

Importar `ServicesCard` y renderizarla junto a `UnitsCard` (pasando el mismo `snap`).

- [ ] **Step 4: Compilar el panel y verificar en el navegador**

```bash
cd apps/panel && npm run build
git add -f apps/panel/dist && git commit -m "feat(panel): tarjeta Servicios + enlace a Cockpit"
./scripts/deploy-relay.sh
```

Abrir `http://atlas-hub.local:8765/panel/`: la tarjeta Servicios muestra Nextcloud/Jellyfin y el enlace a Cockpit. En la app Tauri (`apps/panel-app`), si el enlace no abre, conceder `shell.allow-open` a `https://atlas-hub:9090` en `apps/panel-app/src-tauri/capabilities/default.json`.

- [ ] **Step 5: Verificar los dos builds**

```bash
cd apps/panel-app && npm run build:ui   # base ./ (dist NO se commitea)
```

Expected: misma tarjeta en la app de barra de menús.

---

### Task 15: Documentación, verificación global y registro

**Files:**
- Modify: `docs/relay.md` (ufw ahora activo; perímetro con tailnet)
- Modify: `docs/implemented.md` (entrada con los commits)
- Modify: `docs/index.md` (si cambia algo)

- [ ] **Step 1: Actualizar `docs/relay.md`**

Corregir la nota de "ufw INACTIVO" (ahora está activo), documentar `RELAY_EXTRA_HOSTS`/`RELAY_ALLOW_NETWORKS`/`CALDAV_URL`/`PANEL_SERVICE_PROBES` en el resumen de endpoints/entorno, y añadir Tailscale al manual de operaciones (sección nueva, con "mantener parcheado").

- [ ] **Step 2: Añadir a `docs/implemented.md`**

Entrada con el nombre exacto de cada commit y una línea por fase (tailscale, cockpit, podman, nextcloud, jellyfin, calendario, panel).

- [ ] **Step 3: Verificación global de la spec (§14)**

Ejecutar los 7 criterios de la spec §14 (acceso desde iPhone en datos móviles, Nextcloud subir/descargar + evento, Jellyfin reproducir, `calendar_read`/`calendar_write`, tarjeta Servicios, terminal de Cockpit, `/v1/health` ok). Registrar resultados.

- [ ] **Step 4: Commitear docs (permiso del usuario)**

```bash
git add docs/relay.md docs/implemented.md
git commit -m "docs(atlas-hub): operación del servidor de servicios y perímetro"
```

---

## Self-review

- **Spec coverage:** decisiones 1-12 de la spec ↔ tareas: Cockpit (T6), Tailscale (T1-4), Podman+Quadlet (T7-10), SSD (T7-10 volúmenes), DERP (automático), backup ⏸️ (sin tarea, pendiente documentado en spec §11), calendario (T11-12), Hub Panel (T13-14), MagicDNS (Tailscale), MariaDB (T8), HTTP sobre túnel (T9-10), migrar iCloud (T11).
- **Pendiente consciente:** backup (decisión ⏸️) sin tarea — queda en la spec §11, no en este plan.
- **Placeholders:** los únicos literales `<generar>`, `TU_TOKEN`, `100.64.0.1` son secretos/IP que se rellenan en ejecución (no placeholders de plan).
- **Type consistency:** `client_allowed(client_ip, networks)` definida en T5 y usada solo ahí; `caldav_base()` en T12; `service_probes`/`_parse_probes`/`_services` en T13 coherentes con el test; `snapshot.services` (T13) consumido por `ServicesCard` (T14) con el tipo `services: {name,url,up}[]` en `api.ts`.
