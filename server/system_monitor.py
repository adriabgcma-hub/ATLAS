#!/usr/bin/env python3
"""Observador del sistema para el panel ATLAS (solo-homelab).

- Recopila el estado de las unidades systemd relevantes, puertos a la escucha,
  contenedores Docker, la nube (Nextcloud + MariaDB desplegada con compose),
  Tailscale y los recursos del equipo. Todo best-effort: si una fuente no
  responde, el campo queda `null` y el resto del snapshot sigue en pie.
- Guarda la lógica de acciones del panel (start/stop/restart) tras la doble
  confirmación: prohíbe verbos y unidades fuera de la lista blanca inyectada
  por entorno, y las ejecuta vía `systemctl` (polkit limita las de sistema) y
  `sudo -n` sobre un ayudante con raíz fija (`/usr/local/lib/atlas/panel-docker.sh`).
- Solo stdlib, sin dependencias. Python >= 3.11.

Variables de entorno (valores por defecto pensados en homelab):
  PANEL_SYSTEM_UNITS  unidades systemd de nivel de sistema para mostrar.
  PANEL_ACTION_UNITS  unidades de sistema ACCIONABLES (lista blanca).
  PANEL_USER_UNITS    unidades systemd ACCIONABLES de usuario.
  PANEL_CLOUD_COMPOSE ruta del docker-compose.yml de la nube.
  PANEL_CLOUD_URL     probe local de Nextcloud (status.php).
  PANEL_DIST_DIR      raíz estática del panel (apps/panel/dist).
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import threading
import time
import urllib.error
import urllib.request

SNAPSHOT_TTL_S = 10
SUBPROC_TIMEOUT_S = 6
CLOUD_PROBE_TIMEOUT_S = 5
OP_TTL_S = 30
ACTION_VERBS = ("start", "stop", "restart")
UNIT_RE = re.compile(r"^[A-Za-z0-9@._~-]+\.service$")
COMPOSE_TARGETS = ("app", "db")
HELPER = "/usr/local/lib/atlas/panel-docker.sh"

DEFAULT_SYSTEM_UNITS = (
    "docker.service,ssh.service,tailscaled.service,cups.service,"
    "avahi-daemon.service,macfan.service,lid-monitor.service,"
    "smartmontools.service,NetworkManager.service"
)
DEFAULT_ACTION_UNITS = (
    "docker.service,cups.service,avahi-daemon.service,macfan.service,"
    "lid-monitor.service,smartmontools.service"
)
DEFAULT_USER_UNITS = "atlas-relay.service"

KNOWN_PORTS = {"22": "ssh", "631": "cups", "8765": "atlas-relay",
               "8080": "nextcloud (compose)", "443": "tailscale serve"}


def _env_list(name: str, default: str) -> list[str]:
    return [c.strip() for c in
            os.environ.get(name, default).split(",") if c.strip()]


def _run(cmd: list[str], timeout: int = SUBPROC_TIMEOUT_S
         ) -> tuple[int, str, str] | None:
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return proc.returncode, proc.stdout, proc.stderr
    except (OSError, subprocess.TimeoutExpired, ValueError):
        return None


def _parse_since(raw: str) -> str:
    return raw.strip() if raw.strip() else ""


class Ops:
    """Registro de acciones a doble confirmación (patrón de `confirmations`).

    `create()` devuelve el id; `find()` lo devuelve sin expirar; resolve()
    borra (consume) la operación, así que un op-id vale exactamente una vez.
    """

    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.items: dict[str, dict] = {}

    def create(self, kind: str, summary: str, detail: dict) -> str:
        op_id = "op-" + os.urandom(6).hex()
        now = time.time()
        with self.lock:
            for key in [k for k, v in self.items.items()
                        if now - v["created"] > OP_TTL_S]:
                self.items.pop(key, None)
            self.items[op_id] = {"kind": kind, "summary": summary,
                                 "detail": detail, "created": now}
        return op_id

    def find(self, op_id: str) -> dict | None:
        now = time.time()
        with self.lock:
            entry = self.items.get(op_id)
            if entry is None or now - entry["created"] > OP_TTL_S:
                self.items.pop(op_id, None)
                return None
            return dict(entry)

    def consume(self, op_id: str) -> dict | None:
        with self.lock:
            entry = self.items.pop(op_id, None)
        if entry is None or time.time() - entry["created"] > OP_TTL_S:
            return None
        return entry


class SystemMonitor:
    """Snapshot del sistema + acciones autorizadas del panel."""

    def __init__(self, log_panel=None) -> None:
        self.system_units = _env_list("PANEL_SYSTEM_UNITS", DEFAULT_SYSTEM_UNITS)
        self.action_units = set(_env_list("PANEL_ACTION_UNITS", DEFAULT_ACTION_UNITS))
        self.user_units = set(_env_list("PANEL_USER_UNITS", DEFAULT_USER_UNITS))
        self.compose = os.environ.get("PANEL_CLOUD_COMPOSE",
                                      "/home/adria/nextcloud/docker-compose.yml")
        self.cloud_url = os.environ.get("PANEL_CLOUD_URL",
                                        "http://127.0.0.1:8080/status.php")
        self.panel_dist = os.path.realpath(os.environ.get(
            "PANEL_DIST_DIR", os.path.join("apps", "panel", "dist")))
        self.log_panel = log_panel  # callable(kind, message)
        self.ops = Ops()
        self._cache: tuple[float, dict] | None = None
        self._cache_lock = threading.Lock()

    # -- utilidades --------------------------------------------------------
    def _panel_log(self, kind: str, message: str) -> None:
        if self.log_panel is not None:
            self.log_panel(kind, message)

    def can_act(self, unit: str, verb: str) -> tuple[bool, str]:
        """(permitido, alcance) para una acción de unidad systemd."""
        if verb not in ACTION_VERBS or not UNIT_RE.match(unit):
            return (False, "")
        if unit in self.user_units:
            return (True, "user")
        if unit in self.action_units:
            return (True, "system")
        return (False, "")

    # -- snapshot: unidades ------------------------------------------------
    _UNIT_KEYS = ("Description", "LoadState", "ActiveState", "SubState",
                  "ActiveEnterTimestamp", "MainPID", "MemoryCurrent")

    def _unit_info(self, unit: str, scope: str) -> dict:
        base = ["systemctl", "--user"] if scope == "user" else ["systemctl"]
        props = [f"-p{k}" for k in self._UNIT_KEYS]
        result = _run(base + ["show", unit, "--no-pager", *props])
        info: dict = {k: "" for k in self._UNIT_KEYS}
        if result is None:
            return {"unit": unit, "scope": scope, "error": "no_responde"}
        current = None
        for line in result[1].splitlines():
            key, sep, value = line.partition("=")
            if sep and key in info:  # propiedad nueva (continuaciones caen aquí)
                current = key
                info[key] = value
            elif current is not None:
                info[current] += " " + line.strip()
        desc, load, active, sub, since, pid, mem = (
            info["Description"], info["LoadState"], info["ActiveState"],
            info["SubState"], info["ActiveEnterTimestamp"],
            info["MainPID"], info["MemoryCurrent"])
        return {
            "unit": unit, "scope": scope,
            "description": desc or None,
            "load": load or None, "active": active or None,
            "sub": sub or None, "since": _parse_since(since) or None,
            "pid": int(pid) if pid.isdigit() else None,
            "mem_bytes": int(mem) if mem.isdigit() else None,
            "actionable": (scope == "user" and unit in self.user_units)
                          or unit in self.action_units,
        }

    def _units(self) -> list[dict]:
        out = [self._unit_info(u, "system") for u in self.system_units]
        out.extend(self._unit_info(u, "user") for u in self.user_units)
        return out

    # -- snapshot: puertos ---------------------------------------------------
    def _ports(self) -> list[dict]:
        result = _run(["ss", "-tlnH"])
        if result is None:
            return []
        found: dict[str, dict] = {}
        for line in result[1].splitlines():
            parts = line.split()
            if len(parts) < 4:
                continue
            local = parts[3]
            port = local.rpartition(":")[2]
            if not port.isdigit():
                continue
            found.setdefault(port, {"port": int(port),
                                    "address": local.rpartition(":")[0],
                                    "service": KNOWN_PORTS.get(port)})
        return [found[p] for p in sorted(found, key=int)]

    # -- snapshot: docker ----------------------------------------------------
    def _docker(self) -> dict:
        result = _run(["sudo", "-n", HELPER, "ps"])
        if result is None or result[0] != 0:
            return {"available": False, "containers": []}
        containers = []
        for line in result[1].splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                raw = json.loads(line)
            except ValueError:
                continue
            labels = raw.get("Labels") or {}
            containers.append({
                "id": (raw.get("ID") or "")[:12],
                "name": raw.get("Names"),
                "image": raw.get("Image"),
                "state": raw.get("State"),
                "status": raw.get("Status"),
                "compose_service": labels.get("com.docker.compose.service"),
                "compose_project": labels.get("com.docker.compose.project"),
            })
        return {"available": True, "containers": containers}

    # -- snapshot: nube (nextcloud) ------------------------------------------
    def _nextcloud(self) -> dict:
        info: dict = {"status": None, "version": None, "error": None}
        try:
            with urllib.request.urlopen(self.cloud_url,
                                        timeout=CLOUD_PROBE_TIMEOUT_S) as resp:
                body = json.loads(resp.read().decode("utf-8", "replace"))
            info["status"] = "ok" if body.get("installed") else "ok?" 
            info["version"] = body.get("version")
            info["maintainer_mode"] = body.get("maintenance", False)
        except (urllib.error.URLError, ValueError, OSError) as exc:
            info["status"] = "down"
            info["error"] = type(exc).__name__
        return info

    def _tailscale_serve(self) -> dict:
        result = _run(["tailscale", "serve", "status"], timeout=4)
        if result is None:
            return {"url": None, "raw": None}
        _rc, out, _err = result
        url = next((ln.strip().rstrip(":") for ln in out.splitlines()
                    if ln.strip().startswith("https://")), None)
        return {"url": url, "raw": out.strip() or None}

    # -- snapshot: recursos ---------------------------------------------------
    def _resources(self) -> dict:
        data: dict = {}
        try:
            with open("/proc/meminfo", encoding="utf-8") as fh:
                info = dict(
                    (k.rstrip(":"), int(v.split()[0]))
                    for k, v in (ln.split(None, 1) for ln in fh if len(ln.split(None, 1)) == 2)
                )
            total = info.get("MemTotal", 0)
            avail = info.get("MemAvailable", 0)
            data["mem_total_kb"], data["mem_available_kb"] = total, avail
            data["mem_used_pct"] = round(100 * (total - avail) / total) if total else None
        except (OSError, ValueError):
            pass
        try:
            data["disk_root"] = {"total": shutil.disk_usage("/").total,
                                 "free": shutil.disk_usage("/").free}
        except OSError:
            data["disk_root"] = None
        try:
            with open("/proc/uptime", encoding="utf-8") as fh:
                data["uptime_s"] = int(float(fh.read().split()[0]))
        except (OSError, ValueError, IndexError):
            pass
        try:
            with open("/proc/loadavg", encoding="utf-8") as fh:
                data["load_1"] = float(fh.read().split()[0])
        except (OSError, ValueError, IndexError):
            pass
        temps = []
        try:
            base = "/sys/class/hwmon"
            for name in sorted(os.listdir(base)):
                folder = os.path.join(base, name)
                if not os.path.isdir(folder):
                    continue
                label = None
                for lfile in sorted(os.listdir(folder)):
                    if lfile.endswith("_label"):
                        try:
                            with open(os.path.join(folder, lfile), encoding="utf-8") as fh:
                                label = fh.read().strip()
                        except OSError:
                            pass
                    if lfile.startswith("temp") and lfile.endswith("_input"):
                        try:
                            with open(os.path.join(folder, lfile), encoding="utf-8") as fh:
                                temps.append({"src": name, "label": label,
                                              "celsius": int(fh.read().strip()) / 1000.0})
                        except (OSError, ValueError):
                            pass
        except OSError:
            pass
        data["temps"] = temps[:6]
        data["temp_c"] = max((t["celsius"] for t in temps), default=None)
        # Batería (es un MacBook).
        try:
            bat = "/sys/class/power_supply/BAT0"
            data["battery"] = {
                "cap_pct": int(open(os.path.join(bat, "capacity"),
                                    encoding="utf-8").read().strip()),
                "status": open(os.path.join(bat, "status"),
                               encoding="utf-8").read().strip(),
            }
        except (OSError, ValueError):
            data["battery"] = None
        try:
            online = self._ac_online()
            data["ac_connected"] = online
        except OSError:
            data["ac_connected"] = None
        return data

    @staticmethod
    def _ac_online() -> bool | None:
        base = "/sys/class/power_supply"
        for name in sorted(os.listdir(base)):
            if not name.startswith(("AC", "ADP")):
                continue
            try:
                with open(os.path.join(base, name, "online"), encoding="utf-8") as fh:
                    value = fh.read().strip()
                if value.isdigit():
                    return value == "1"
            except OSError:
                continue
        return None

    # -- actions ------------------------------------------------------------
    def _helper(self, op: str) -> tuple[int, str, str] | None:
        if not os.access(HELPER, os.X_OK):
            return None
        return _run(["sudo", "-n", HELPER, op], timeout=90)

    def run_service_action(self, unit: str, verb: str) -> dict:
        ok, scope = self.can_act(unit, verb)
        if not ok:
            return {"ok": False, "error": "unidad/verbo no permitido", "scope": ""}
        base = ["systemctl", "--user"] if scope == "user" else ["systemctl"]
        result = _run(base + [verb, unit], timeout=90)
        if result is None:
            return {"ok": False, "error": "systemctl no disponible", "scope": scope}
        return {"ok": result[0] == 0, "rc": result[0], "scope": scope,
                "output": (result[2].strip() or result[1].strip())[:300]}

    def run_compose_action(self, target: str, verb: str) -> dict:
        if target not in COMPOSE_TARGETS or verb not in ACTION_VERBS:
            return {"ok": False, "error": "target/verbo no permitido"}
        result = self._helper(f"compose_{verb}_{target}")
        if result is None:
            return {"ok": False, "error": "ayudante sudo no instalado"}
        return {"ok": result[0] == 0, "rc": result[0],
                "output": (result[2].strip() or result[1].strip())[:300]}

    # -- snapshot completo -----------------------------------------------------
    def snapshot(self, force: bool = False) -> dict:
        with self._cache_lock:
            cached = self._cache
            if cached is not None and not force and time.time() - cached[0] < SNAPSHOT_TTL_S:
                return cached[1]
        units = self._units()
        docker = self._docker()
        cloud_containers = [c for c in docker.get("containers", [])
                            if (c.get("compose_project") or "") == "nextcloud"]
        cloud = {"compose_project": "nextcloud",
                 "containers": cloud_containers,
                 "nextcloud": self._nextcloud(),
                 "tailscale_serve": self._tailscale_serve()}
        data = {
            "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "units": units,
            "ports": self._ports(),
            "docker": docker,
            "cloud": cloud,
            "resources": self._resources(),
            "actions": {
                "verbs": list(ACTION_VERBS),
                "system_units": sorted(self.action_units),
                "user_units": sorted(self.user_units),
                "compose_targets": list(COMPOSE_TARGETS),
            },
        }
        with self._cache_lock:
            self._cache = (time.time(), data)
        return data
