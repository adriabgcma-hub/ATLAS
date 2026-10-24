#!/usr/bin/env python3
"""Observador del sistema para el panel ATLAS (atlas-hub).

- Recopila el estado de las unidades systemd de sistema relevantes, los
  puertos a la escucha y los recursos del equipo. Todo best-effort: si una
  fuente no responde, el campo queda `null` y el resto del snapshot sigue
  en pie.
- Guarda la lógica de acciones del panel (start/stop/restart) tras la doble
  confirmación: prohíbe verbos y unidades fuera de la lista blanca inyectada
  por entorno, y las ejecuta vía `systemctl` (polkit limita las de sistema).
  Por ahora la lista blanca está VACÍA en atlas-hub (sin reglas polkit para
  `atlas`): se rellena por entorno a medida que se integren unidades.
- Docker / Nextcloud / Tailscale NO se observan: no existen aún en
  atlas-hub; volverán al panel cuando se integren de verdad.
- Solo stdlib, sin dependencias. Python >= 3.11.

Variables de entorno:
  PANEL_SYSTEM_UNITS  unidades systemd de nivel de sistema para mostrar.
  PANEL_ACTION_UNITS  unidades de sistema ACCIONABLES (lista blanca; vacía
                      por defecto hasta instalar reglas polkit en atlas-hub).
  PANEL_DIST_DIR      raíz estática del panel (apps/panel/dist).
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import threading
import time

SNAPSHOT_TTL_S = 10
SUBPROC_TIMEOUT_S = 6
OP_TTL_S = 30
ACTION_VERBS = ("start", "stop", "restart")
UNIT_RE = re.compile(r"^[A-Za-z0-9@._~-]+\.service$")
COMPOSE_TARGETS = ("app", "db")
HELPER = "/usr/local/lib/atlas/panel-docker.sh"

DEFAULT_SYSTEM_UNITS = ("ssh.service,avahi-daemon.service,ufw.service,"
                        "atlas-relay.service")
# Lista blanca de acciones VACÍA en atlas-hub: sin reglas polkit para el
# usuario `atlas`, ninguna acción `systemctl` del relay puede prosperar.
# Se rellena por entorno (PANEL_ACTION_UNITS) a medida que se integren
# unidades + permisos.
DEFAULT_ACTION_UNITS = ""

KNOWN_PORTS = {"22": "ssh", "8765": "atlas-relay"}


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
        if unit in self.action_units:
            return (True, "system")
        return (False, "")

    # -- snapshot: unidades ------------------------------------------------
    _UNIT_KEYS = ("Description", "LoadState", "ActiveState", "SubState",
                  "ActiveEnterTimestamp", "MainPID", "MemoryCurrent")

    def _unit_info(self, unit: str) -> dict:
        # Caso especial: ufw status es la única fuente de verdad, el servicio es solo un loader
        if unit == "ufw.service":
            # Intentamos con sudo -n para no bloquear el relay
            res = _run(["sudo", "-n", "ufw", "status"])
            if res is None or res[0] != 0:
                # Fallback: intentar ruta absoluta común en Debian
                res = _run(["sudo", "-n", "/usr/sbin/ufw", "status"])
            
            is_active = (res and "Status: active" in res[1])
            return {
                "unit": unit,
                "description": "Uncomplicated Firewall",
                "load": "loaded" if is_active else "not-found",
                "active": "active" if is_active else "inactive",
                "sub": "running" if is_active else "dead",
                "since": None,
                "pid": None,
                "mem_bytes": None,
                "actionable": False,
            }

        props = [f"-p{k}" for k in self._UNIT_KEYS]
        result = _run(["systemctl", "show", unit, "--no-pager", *props])
        info: dict = {k: "" for k in self._UNIT_KEYS}
        if result is None:
            return {"unit": unit, "error": "no_responde"}
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
            "unit": unit,
            "description": desc or None,
            "load": load or None, "active": active or None,
            "sub": sub or None, "since": _parse_since(since) or None,
            "pid": int(pid) if pid.isdigit() else None,
            "mem_bytes": int(mem) if mem.isdigit() else None,
            "actionable": unit in self.action_units,
        }

    def _units(self) -> list[dict]:
        return [self._unit_info(u) for u in self.system_units]

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
    def run_service_action(self, unit: str, verb: str) -> dict:
        ok, _scope = self.can_act(unit, verb)
        if not ok:
            return {"ok": False, "error": "unidad/verbo no permitido"}
        result = _run(["systemctl", verb, unit], timeout=90)
        if result is None:
            return {"ok": False, "error": "systemctl no disponible"}
        return {"ok": result[0] == 0, "rc": result[0],
                "output": (result[2].strip() or result[1].strip())[:300]}

    # -- snapshot completo -----------------------------------------------------
    def snapshot(self, force: bool = False) -> dict:
        with self._cache_lock:
            cached = self._cache
            if cached is not None and not force and time.time() - cached[0] < SNAPSHOT_TTL_S:
                return cached[1]
        data = {
            "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "units": self._units(),
            "ports": self._ports(),
            "resources": self._resources(),
            "actions": {
                "verbs": list(ACTION_VERBS),
                "system_units": sorted(self.action_units),
            },
        }
        with self._cache_lock:
            self._cache = (time.time(), data)
        return data
