#!/usr/bin/env python3
"""Watchdog del relay ATLAS: lo reinicia si deja de responder.

Solo biblioteca estándar. Pensado para ejecutarse desde `atlas-health.timer`
(cada 60 s) como root en atlas-hub:

    /usr/bin/python3 /opt/atlas/repo/deploy/server/atlas-healthcheck.py

El relay solo escucha en la IP LAN del servidor (nunca en loopback), así que la
URL por defecto se autodetecta igual que hace el propio relay (un `connect` UDP
no envía tráfico). Tras N fallos consecutivos reinicia `atlas-relay.service`.

Variables de entorno (todas opcionales):

    ATLAS_HEALTH_URL        URL completa a comprobar (omite la autodetección)
    ATLAS_HEALTH_PORT       puerto del relay (defecto: 8765)
    ATLAS_HEALTH_TIMEOUT    timeout de la petición, segundos (defecto: 3)
    ATLAS_HEALTH_THRESHOLD  fallos consecutivos antes de reiniciar (defecto: 2)
    ATLAS_HEALTH_STATE_DIR  directorio del contador (defecto: /var/lib/atlas-health)
    ATLAS_HEALTH_UNIT       unidad a reiniciar (defecto: atlas-relay)
    ATLAS_HEALTH_DRY_RUN    "1" no reinicia: solo registra lo que haría

Códigos de salida: 0 normal (sano, fallo aislado o reinicio correcto), 1 error
al reiniciar o fallo inesperado del propio chequeo.
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

PORT = int(os.environ.get("ATLAS_HEALTH_PORT", "8765"))
TIMEOUT = float(os.environ.get("ATLAS_HEALTH_TIMEOUT", "3"))
THRESHOLD = max(1, int(os.environ.get("ATLAS_HEALTH_THRESHOLD", "2")))
UNIT = os.environ.get("ATLAS_HEALTH_UNIT", "atlas-relay")
DRY_RUN = os.environ.get("ATLAS_HEALTH_DRY_RUN") == "1"
SYSTEMCTL = "/usr/bin/systemctl"


def log(message: str) -> None:
    print(f"atlas-health: {message}", flush=True)


def lan_ip() -> str:
    """IP LAN sin enviar tráfico (mismo truco que el relay)."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        sock.connect(("10.255.255.1", 1))
        return sock.getsockname()[0]
    except OSError:
        return "127.0.0.1"
    finally:
        sock.close()


def health_url() -> str:
    explicit = os.environ.get("ATLAS_HEALTH_URL")
    if explicit:
        return explicit
    return f"http://{lan_ip()}:{PORT}/v1/health"


def state_dir() -> Path:
    """Directorio escribible para el contador (systemd usa StateDirectory)."""
    configured = os.environ.get("ATLAS_HEALTH_STATE_DIR")
    if configured:
        path = Path(configured)
    else:
        path = Path("/var/lib/atlas-health")
    try:
        path.mkdir(parents=True, exist_ok=True)
        probe = path / ".write-test"
        probe.write_text("", encoding="utf-8")
        probe.unlink()
        return path
    except OSError:
        # Fuera de systemd (pruebas locales) el estado va a un temporal.
        fallback = Path(tempfile.gettempdir()) / "atlas-health"
        fallback.mkdir(parents=True, exist_ok=True)
        return fallback


def read_failures(path: Path) -> int:
    try:
        return max(0, int(path.read_text(encoding="utf-8").strip() or "0"))
    except (OSError, ValueError):
        return 0


def write_failures(path: Path, value: int) -> None:
    try:
        path.write_text(f"{value}\n", encoding="utf-8")
    except OSError as exc:
        log(f"no se pudo guardar el contador en {path}: {exc}")


def relay_healthy(url: str) -> bool:
    """True solo si responde 200 y `{"ok": true}`."""
    try:
        with urllib.request.urlopen(url, timeout=TIMEOUT) as resp:
            if resp.status != 200:
                log(f"respuesta HTTP {resp.status} en {url}")
                return False
            payload = json.load(resp)
    except (urllib.error.URLError, OSError, ValueError) as exc:
        log(f"sin respuesta válida de {url}: {exc}")
        return False
    if payload.get("ok") is not True:
        log(f"{url} responde pero sin ok=true: {payload}")
        return False
    return True


def service_state() -> str:
    try:
        result = subprocess.run(
            [SYSTEMCTL, "is-active", UNIT],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        return result.stdout.strip() or "unknown"
    except (OSError, subprocess.SubprocessError) as exc:
        log(f"no se pudo consultar el estado de {UNIT}: {exc}")
        return "unknown"


def restart_service() -> bool:
    if DRY_RUN:
        log(f"DRY RUN: reiniciaría {UNIT} (no se ejecuta)")
        return True
    log(f"reiniciando {UNIT} tras {THRESHOLD} fallos consecutivos")
    try:
        result = subprocess.run(
            [SYSTEMCTL, "restart", UNIT],
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        log(f"fallo al reiniciar {UNIT}: {exc}")
        return False
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        log(f"systemctl restart {UNIT} devolvió {result.returncode}: {detail}")
        return False
    state = service_state()
    log(f"{UNIT} reiniciado; estado: {state}")
    return state == "active"


def main() -> int:
    url = health_url()
    directory = state_dir()
    counter = directory / "failures"

    if relay_healthy(url):
        if read_failures(counter) > 0:
            log(f"{url} vuelve a responder; contador a cero")
            write_failures(counter, 0)
        else:
            log(f"{url} OK")
        return 0

    failures = read_failures(counter) + 1
    write_failures(counter, failures)

    state = service_state()
    if state in ("activating", "deactivating"):
        # Un reinicio en curso no se pisa: se reintenta en la siguiente pasada.
        log(f"fallo {failures}/{THRESHOLD}; {UNIT} está {state}, espero")
        return 0

    if failures < THRESHOLD:
        log(f"fallo {failures}/{THRESHOLD}; aún no reinicio")
        return 0

    ok = restart_service()
    # Se reinicia el contador aunque falle: si el servicio no levanta, el
    # siguiente intento espera otra tanda en vez de martillear cada minuto.
    write_failures(counter, 0)
    if ok:
        return 0
    log("el relay sigue sin recuperarse; revisa journalctl -u atlas-relay")
    return 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:  # pragma: no cover - solo uso interactivo
        sys.exit(130)
