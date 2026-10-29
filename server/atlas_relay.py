#!/usr/bin/env python3
"""Servidor de Atlas (Fase 3): el cerebro vive aquí, 24/7, app abierta o no.

- Consulta el bot por **polling saliente** (`getUpdates`): nunca abre webhooks,
  no se expone a internet. Responde a Telegram por `sendMessage` directo.
- Dueño único de sesiones e historial (persistidos en disco).
- Llama a Groq con 3 API keys rotativas (`openai/gpt-oss-120b` por defecto).
  Sin system prompt. Sin streaming en v1.
- Un worker único procesa peticiones en FIFO estricto (Telegram + input local).
- Sirve eventos por HTTP **solo en la red local** para que la app presente
  la conversación cuando está abierta.
- Solo stdlib, sin dependencias. Requiere Python >= 3.11.
- Ejecutar desde la raíz del repo (usa `temp/` para estado, ignorado por Git).

Endpoints (todos JSON, todos solo red local):
  GET  /v1/health               {ok, telegram, llm, pending}
  GET  /v1/events?since=N       {events: [{seq, kind, ...}]} (kind: message|state)
  GET  /v1/session               {sessionId, state, messages} | null
  POST /v1/send    {content, source?} -> {ok, requestId, sessionId}
                   (lo que empieza por "/" es un comando local, sin Groq)
  POST /v1/cancel {requestId}   -> {ok, cancelled}

Variables de entorno (valores solo en `.env` local, nunca en Git):
  TELEGRAM_BOT_TOKEN         Token de @BotFather. Vacío = Telegram desactivado.
  TELEGRAM_ALLOWED_USER_IDS  IDs separados por comas. Solo esos hablan al bot.
  GROQ_API_KEY_1..3          Claves rotativas. Vacías = stub local explícito.
  GROQ_MODEL                 Modelo (defecto openai/gpt-oss-120b).
  GROQ_TEMPERATURE            Temperatura 0..2 (defecto 0.7).
  RELAY_TIMEZONE             Zona horaria del contexto (defecto Europe/Madrid).
  RELAY_HOST                 IP de escucha. Vacío = IP LAN autodetectada.
  RELAY_PORT                 Puerto (defecto 8765).
  RELAY_DATA_DIR             Estado en disco (defecto temp/atlas-relay, dentro
                             del directorio de trabajo).
  RELAY_CORS_ORIGIN          Origen CORS (defecto vacío = sin CORS). Solo dev.
  CALDAV_EDIT_CALENDARS      Nombres con permiso de escritura, separados por
                             comas (defecto: Exámenes,Celebraciones,Horario,
                             Deberes). Solo lectura en el resto.

Códigos de salida: 0 ok, 1 fallo en ejecución, 2 error de configuración.
"""

from __future__ import annotations

import ipaddress
import json
import os
import queue
import socket
import sqlite3
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import zoneinfo
from collections import deque
from contextlib import ExitStack, contextmanager
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import NoReturn

import atlas_caldav
import hefesto
import system_monitor

MAX_CONTENT_LEN = 4000
MAX_BODY_BYTES = 64 * 1024
TELEGRAM_POLL_TIMEOUT_S = 25
TELEGRAM_RETRY_S = 5
GROQ_TIMEOUT_S = 25
OP_TIMEOUT_S = 30
MAX_HISTORY = 6
STORE_HISTORY = 100
DEFAULT_TIMEZONE = "Europe/Madrid"

# Sistema fijo (cacheable: no cambia entre llamadas) y contexto variable.
SYSTEM_FIXED = (
    "Eres Atlas, un asistente personal que acompaña al usuario desde su servidor. "
    "Responde siempre en el idioma en que te escriban (por defecto, español). "
    "Tono cercano pero sin tutear: hable de usted, directo y sin rodeos ni formalismos innecesarios. "
    "Si no sabes algo, dilo. Conciso por defecto; detalle solo si lo piden. "
    "Reproduce los nombres propios (calendarios, personas) literalmente, "
    "sin abreviarlos ni alterarlos. Reporte el resultado de las herramientas "
    "tal cual, sin reinterpretarlo ni inventar motivos. "
    "Si el canal actual es Telegram, sé más breve. "
    "Tienes herramientas de calendario: úsala cuando pregunten por eventos "
    "en vez de inventar."
)

_DIAS = ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"]
_MESES = ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio",
          "agosto", "septiembre", "octubre", "noviembre", "diciembre"]
DEFAULT_GROQ_MODEL = "openai/gpt-oss-120b"
GROQ_API = "https://api.groq.com/openai/v1/chat/completions"
TELEGRAM_API = "https://api.telegram.org"
# Groq (vía Cloudflare) rechaza el User-Agent por defecto de urllib con 403.
USER_AGENT = "AtlasRelay/3"

TERMINAL_STATES = {"idle", "cancelled", "completed", "failed"}

CONFIRM_TIMEOUT_S = 300
MAX_TURNS = 4

# Palabras que cuentan como confirmación explícita desde Telegram (texto libre).
CONFIRM_WORDS = {
    "sí", "si", "sií", "síí", "adelante", "vale", "ok", "okey", "oké",
    "confirmo", "confirmar", "ejecutar", "hazlo", "dále", "dale", "vamos",
    "sí adelante", "si adelante", "confirmado", "acepto", "autorizo",
}
# Palabras que cuentan como denegación explícita desde Telegram.
DENY_WORDS = {
    "no", "no quiero", "no gracias", "cancela", "cancelar", "stop",
    "detener", "parar", "olvídalo", "olvidalo", "déjalo", "dejalo",
    "no ejecutes", "no hacerlo", "deniego", "rechazo",
}

EDITABLE_DEFAULT = "Exámenes,Celebraciones,Horario,Deberes"

# Comandos locales (sin Groq). Las claves van en minúsculas y sin parámetros.
COMMAND_ALIASES = {
    "/clear": "clear", "/limpiar": "clear",
    "/help": "help", "/ayuda": "help",
    "/log": "log", "/registro": "log",
}
LOG_INTERACTIONS = 5

# Catálogo de herramientas (ver docs/tools.md). El riesgo decide si la
# ejecución es automática (read) o con confirmación en UI (write).
TOOL_DEFS = [
    {
        "id": "calendar_read",
        "risk": "read",
        "requiresConfirmation": False,
        "description": "Lee próximos eventos del calendario iCloud.",
        "schema": {
            "type": "object",
            "properties": {
                "days": {"type": "integer", "description": "días hacia adelante (1..31)"},
                "calendar": {"type": "string", "description": "nombre del calendario (opcional)"},
            },
        },
    },
    {
        "id": "calendar_write",
        "risk": "write",
        "requiresConfirmation": True,
        "description": ("Crea, actualiza o borra eventos. SOLO en calendarios editables; "
                        "cualquier otro se rechaza."),
        "schema": {
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["create", "update", "delete"]},
                "calendar": {"type": "string", "description": "nombre del calendario"},
                "uid": {"type": "string", "description": "requerido para update/delete"},
                "summary": {"type": "string", "description": "título"},
                "start": {"type": "string", "description": "YYYY-MM-DD o YYYY-MM-DDTHH:MM"},
                "end": {"type": "string", "description": "fin (mismo formato)"},
                "description": {"type": "string"},
                "location": {"type": "string"},
            },
            "required": ["action", "calendar"],
        },
    },
]


HEFESTO_TOOL = {
    "id": "hefesto_echo",
    "risk": "external",
    "requiresConfirmation": False,
    "description": ("Encarga una tarea a Hefesto, el especialista del Mac de Atlas, "
                    "y vuelve al instante con el estado de la delegación. Si el "
                    "encargo se acepta, díselo al usuario y no inventes el "
                    "resultado: llegará aparte como mensaje de Hefesto."),
    "schema": {
        "type": "object",
        "properties": {
            "text": {"type": "string",
                     "description": "tarea para Hefesto (máx. 256 bytes)"},
        },
        "required": ["text"],
    },
}


def norm_name(name: str) -> str:
    return name.strip().casefold()


def tool_payload(defs: list[dict] = TOOL_DEFS) -> list[dict]:
    """Catálogo en formato OpenAI function calling para Groq."""
    return [{"type": "function",
             "function": {"name": t["id"], "description": t["description"],
                          "parameters": t["schema"]}} for t in defs]


def log(message: str) -> None:
    print(f"[relay {time.strftime('%H:%M:%S')}] {message}", file=sys.stderr, flush=True)


def dlog(brain: "Brain", category: str, message: str, **extra) -> None:
    """Log detallado a archivo + stderr, con timestamp y categoría."""
    ts = time.strftime("%Y-%m-%dT%H:%M:%S")
    line = f"{ts} [{category}] {message}"
    if extra:
        line += " " + " ".join(f"{k}={v}" for k, v in extra.items())
    try:
        with open(brain.log_path, "a", encoding="utf-8") as fh:
            fh.write(line + "\n")
            fh.flush()
    except OSError:
        pass
    print(f"[relay {time.strftime('%H:%M:%S')}] {category}: {message}", file=sys.stderr, flush=True)


def fail_config(message: str) -> NoReturn:
    log(f"config: {message}")
    raise SystemExit(2)


def utc_now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def new_id(prefix: str) -> str:
    return f"{prefix}-{os.urandom(8).hex()}"


def one_line(value: str, limit: int) -> str:
    """Texto en una sola línea y acotado (logs y resúmenes de comandos)."""
    text = " ".join(value.split())
    if len(text) <= limit:
        return text
    return text[: limit - 1].rstrip() + "…"


def local_time_label(value: str, timezone_name: str) -> str:
    """`2026-10-26T14:32:00Z` → `26/10 15:32` en la zona configurada."""
    try:
        moment = datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ")
        moment = moment.replace(tzinfo=zoneinfo.ZoneInfo("UTC"))
        return moment.astimezone(zoneinfo.ZoneInfo(timezone_name)).strftime("%d/%m %H:%M")
    except (ValueError, KeyError):
        return value


def read_env() -> dict:
    port_raw = os.environ.get("RELAY_PORT", "8765").strip()
    try:
        port = int(port_raw)
    except ValueError:
        fail_config(f"RELAY_PORT no es un número: {port_raw!r}")
    if not 1 <= port <= 65535:
        fail_config(f"RELAY_PORT fuera de rango: {port}")

    data_dir = os.environ.get("RELAY_DATA_DIR", os.path.join("temp", "atlas-relay"))
    resolved = os.path.realpath(data_dir)
    cwd = os.path.realpath(os.getcwd())
    if resolved != cwd and not resolved.startswith(cwd + os.sep):
        fail_config("RELAY_DATA_DIR debe quedar dentro del directorio de trabajo.")

    allowed: set[int] = set()
    for chunk in os.environ.get("TELEGRAM_ALLOWED_USER_IDS", "").replace(";", ",").split(","):
        chunk = chunk.strip()
        if not chunk:
            continue
        if not chunk.isdigit():
            fail_config("TELEGRAM_ALLOWED_USER_IDS lleva un id no numérico.")
        allowed.add(int(chunk))

    groq_keys = []
    for name in ("GROQ_API_KEY_1", "GROQ_API_KEY_2", "GROQ_API_KEY_3"):
        value = os.environ.get(name, "").strip()
        if value:
            groq_keys.append(value)
    model = os.environ.get("GROQ_MODEL", "").strip() or DEFAULT_GROQ_MODEL
    temp_raw = os.environ.get("GROQ_TEMPERATURE", "0.7").strip()
    try:
        temperature = float(temp_raw)
    except ValueError:
        fail_config(f"GROQ_TEMPERATURE no es un número: {temp_raw!r}")
    if not 0.0 <= temperature <= 2.0:
        fail_config(f"GROQ_TEMPERATURE fuera de rango 0..2: {temp_raw!r}")
    timezone_name = os.environ.get("RELAY_TIMEZONE", "").strip() or DEFAULT_TIMEZONE
    try:
        zoneinfo.ZoneInfo(timezone_name)
    except Exception:
        fail_config(f"RELAY_TIMEZONE desconocida: {timezone_name!r}")

    editable_names = [chunk.strip() for chunk in
                      os.environ.get("CALDAV_EDIT_CALENDARS",
                                     EDITABLE_DEFAULT).split(",") if chunk.strip()]

    return {
        "token": os.environ.get("TELEGRAM_BOT_TOKEN", "").strip(),
        "allowed": allowed,
        "groq_keys": groq_keys,
        "groq_model": model,
        "groq_temperature": temperature,
        "timezone": timezone_name,
        "caldav_user": os.environ.get("CALDAV_USERNAME", "").strip(),
        "caldav_password": os.environ.get("CALDAV_APP_PASSWORD", "").strip(),
        "editable": {norm_name(name) for name in editable_names},
        "editable_names": editable_names,
        "host": os.environ.get("RELAY_HOST", "").strip(),
        "port": port,
        "data_dir": resolved,
        "cors_origin": os.environ.get("RELAY_CORS_ORIGIN", "").strip(),
    }


def lan_ip() -> str:
    """IP LAN sin enviar tráfico (connect UDP no transmite)."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        sock.connect(("10.255.255.1", 1))
        return sock.getsockname()[0]
    except OSError:
        return "127.0.0.1"
    finally:
        sock.close()


def atomic_write(path: str, payload: dict) -> None:
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(payload, fh)
    os.replace(tmp, path)


class Brain:
    """Dueño único de sesiones, eventos, Groq y Telegram."""

    def __init__(self, cfg: dict) -> None:
        self.cfg = cfg
        self.lock = threading.Lock()
        self.events: deque[dict] = deque()
        self.seq = 0
        self.jobs: queue.Queue[dict] = queue.Queue()
        self.cancelled: set[str] = set()
        self.sessions: dict = {}
        self.active_id: str | None = None
        self.offset = 0
        self._caldav: atlas_caldav.CalDAVClient | None = None
        self.confirmations: dict = {}
        self._cal_cache: dict = {}
        # Panel: identidad del bot (getMe) y peticiones en vuelo (cola+activo).
        self.bot_info: dict | None = None
        self.inflight: set[str] = set()
        # Uso y resultado por petición: alimenta /log (se poda, no se persiste).
        self.usage_by_request: dict[str, dict] = {}
        self.result_by_request: dict[str, str] = {}
        # Logging detallado + consumo de tokens (Groq).
        self.log_path = os.path.join(cfg["data_dir"], "relay.log")
        self.token_usage = {"prompt": 0, "completion": 0, "total": 0,
                            "calls": 0, "by_model": {}}
        # Hefesto: journal/controlador en proceso y delegaciones en vuelo.
        self._hefesto_journal: hefesto.Journal | None = None
        self._hefesto_device: str | None = None
        self._delegations: dict[str, dict] = {}
        self._hefesto_stop: threading.Event | None = None
        self._hefesto_thread: threading.Thread | None = None
        self._load()

    # -- persistencia ----------------------------------------------------
    def _paths(self) -> tuple[str, str]:
        data = self.cfg["data_dir"]
        return (os.path.join(data, "sessions.json"), os.path.join(data, "telegram-offset.json"))

    def _load(self) -> None:
        sessions_path, offset_path = self._paths()
        try:
            with open(sessions_path, encoding="utf-8") as fh:
                data = json.load(fh)
            sessions = data.get("sessions", {})
            if isinstance(sessions, dict):
                self.sessions = sessions
            active = data.get("active_id")
            if isinstance(active, str) and active in self.sessions:
                self.active_id = active
            seq = data.get("seq", 0)
            if isinstance(seq, int) and seq >= 0:
                # El cursor sobrevive al reinicio: los clientes no quedan ciegos.
                self.seq = seq
            # Operaciones a medias murieron con el proceso anterior: a reposo.
            for session in self.sessions.values():
                if session.get("state") not in TERMINAL_STATES:
                    session["state"] = "idle"
        except (OSError, ValueError, AttributeError):
            pass
        try:
            with open(offset_path, encoding="utf-8") as fh:
                value = json.load(fh).get("offset", 0)
            self.offset = int(value) if isinstance(value, int) and value >= 0 else 0
        except (OSError, ValueError, AttributeError):
            pass
        self._delegations = self._load_delegations()

    def _save_sessions(self) -> None:
        sessions_path, _ = self._paths()
        # Poda: activa + 19 recientes (cada /clear crea sesión).
        if len(self.sessions) > 20:
            keep = [self.active_id] + [sid for sid in self.sessions
                                       if sid != self.active_id][-19:]
            self.sessions = {sid: self.sessions[sid] for sid in keep
                             if sid in self.sessions}
        atomic_write(sessions_path, {"sessions": self.sessions,
                                     "active_id": self.active_id,
                                     "seq": self.seq})

    def _save_offset(self) -> None:
        _, offset_path = self._paths()
        atomic_write(offset_path, {"offset": self.offset})

    def _delegations_path(self) -> str:
        return os.path.join(self.cfg["data_dir"], "hefesto-delegations.json")

    def _load_delegations(self) -> dict[str, dict]:
        try:
            with open(self._delegations_path(), encoding="utf-8") as fh:
                data = json.load(fh)
            if not isinstance(data, dict):
                return {}
            return {key: entry for key, entry in data.items()
                    if isinstance(entry, dict)
                    and isinstance(entry.get("invocation_id"), str)}
        except (OSError, ValueError, AttributeError):
            return {}

    def _save_delegations(self) -> None:
        atomic_write(self._delegations_path(), self._delegations)

    # -- sesiones --------------------------------------------------------
    def _active(self) -> dict:
        if self.active_id is None or self.active_id not in self.sessions:
            session_id = new_id("ses")
            now = utc_now()
            self.sessions[session_id] = {
                "id": session_id, "state": "idle", "messages": [],
                "created_at": now, "updated_at": now,
            }
            self.active_id = session_id
            self._save_sessions()
        return self.sessions[self.active_id]

    def get_snapshot(self) -> dict | None:
        with self.lock:
            if self.active_id is None:
                return None
            session = self.sessions[self.active_id]
            return {"sessionId": session["id"], "state": session["state"],
                    "messages": list(session["messages"])}

    # -- eventos ---------------------------------------------------------
    def _push_event(self, event: dict) -> None:
        self.seq += 1
        event = dict(event)
        event["seq"] = self.seq
        self.events.append(event)
        if len(self.events) > 500:
            self.events.popleft()

    def _push_state(self, session_id: str, request_id: str, state: str,
                    message: str | None = None) -> None:
        event: dict = {"kind": "state", "type": "session.state_changed",
                       "requestId": request_id, "sessionId": session_id,
                       "timestamp": utc_now(), "state": state}
        if message is not None:
            event["message"] = message
        self._push_event(event)

    def _push_message(self, session_id: str, request_id: str, role: str,
                      content: str, source: str) -> None:
        self._push_event({
            "kind": "message", "type": "conversation.message_appended",
            "requestId": request_id, "sessionId": session_id,
            "timestamp": utc_now(), "role": role, "content": content,
            "source": source,
        })

    def events_since(self, cursor: int) -> list[dict]:
        with self.lock:
            return [e for e in self.events if e["seq"] > cursor]
    def current_seq(self) -> int:
        with self.lock:
            return self.seq

    # -- calendario (solo lectura en v1) ----------------------------------
    def calendar_status(self) -> str:
        if not self.cfg.get("caldav_user") or not self.cfg.get("caldav_password"):
            return "disabled"
        with self.lock:
            return "enabled" if self._caldav is not None else "standby"

    def _caldav_client(self) -> atlas_caldav.CalDAVClient:
        username = self.cfg.get("caldav_user", "")
        password = self.cfg.get("caldav_password", "")
        if not username or not password:
            raise atlas_caldav.CalDAVError("no configurado")
        with self.lock:
            client = self._caldav
        if client is None:
            client = atlas_caldav.CalDAVClient(username, password)
            client.discover()
            with self.lock:
                self._caldav = client
        return client

    def calendar_calendars(self) -> list[dict]:
        try:
            return self._caldav_client().discover()
        except atlas_caldav.CalDAVError:
            with self.lock:
                self._caldav = None  # redescubrir la próxima vez
            raise

    def calendar_upcoming(self, days: int) -> list[dict]:
        # Caché breve: una pregunta repetida no re-golpea iCloud.
        with self.lock:
            cached = self._cal_cache.get(days)
            if cached and time.time() - cached[0] < 120:
                return cached[1]
        try:
            events = self._caldav_client().upcoming(days)
        except atlas_caldav.CalDAVError:
            with self.lock:
                self._caldav = None
            raise
        with self.lock:
            self._cal_cache[days] = (time.time(), events)
        return events

    # -- recepción -------------------------------------------------------
    def _remember(self, request_id: str, usage: dict | None = None,
                  result: str | None = None) -> None:
        """Guarda uso/resultado de una petición. Requiere self.lock."""
        if usage is not None:
            self.usage_by_request[request_id] = usage
        if result is not None:
            self.result_by_request[request_id] = result
        for mapping in (self.usage_by_request, self.result_by_request):
            if len(mapping) > 200:
                keep = list(mapping)[-100:]
                snapshot = {key: mapping[key] for key in keep}
                mapping.clear()
                mapping.update(snapshot)

    def _recent_interactions_locked(self) -> list[dict]:
        """Últimas interacciones de todas las sesiones. Requiere self.lock."""
        interactions = []
        for session in self.sessions.values():
            messages = session.get("messages", [])
            for index, message in enumerate(messages):
                if message.get("role") != "user":
                    continue
                replied = False
                for following in messages[index + 1:]:
                    if following.get("role") == "user":
                        break
                    if following.get("role") == "assistant":
                        replied = True
                        break
                interactions.append({
                    "requestId": message.get("requestId", "?"),
                    "content": message.get("content", ""),
                    "source": message.get("source", "local"),
                    "timestamp": message.get("timestamp", ""),
                    "replied": replied,
                })
        # Reverso antes del sort estable: con el mismo segundo (habitual en
        # comandos), el desempate deja primero la interacción más nueva.
        interactions.reverse()
        interactions.sort(key=lambda item: item["timestamp"], reverse=True)
        return interactions[:LOG_INTERACTIONS]

    def _help_text(self, unknown: str | None = None) -> str:
        editable = ", ".join(self.cfg["editable_names"]) or "ninguno"
        lines = []
        if unknown:
            lines.extend([f"Comando no reconocido: {one_line(unknown, 80)}", ""])
        lines.extend([
            "Comandos",
            "/clear  (/limpiar)  · reinicia la conversación.",
            "/help   (/ayuda)    · esta ayuda.",
            "/log    (/registro) · últimas 5 interacciones y consumo de tokens.",
            "Los comandos responden al instante y no consumen tokens.",
            "",
            "Permisos",
            "· Consultar el calendario es automático, sin confirmación.",
            "· Crear, modificar o borrar eventos pide confirmación.",
            "· La confirmación caduca a los 5 minutos; sin respuesta no se ejecuta.",
            "· En Telegram, «sí» o «adelante» acepta; «no» deniega.",
            f"· Solo se puede escribir en: {editable}.",
        ])
        return "\n".join(lines)

    def _log_text(self) -> str:
        """Resumen ordenado: interacciones y consumo. Requiere self.lock."""
        interactions = self._recent_interactions_locked()
        lines = [f"Últimas {LOG_INTERACTIONS} interacciones", ""]
        if not interactions:
            lines.extend(["Sin interacciones registradas.", ""])
        for index, item in enumerate(interactions, 1):
            request_id = item["requestId"]
            usage = self.usage_by_request.get(request_id)
            result = self.result_by_request.get(request_id)
            if result is None:
                result = "completada" if item["replied"] else "sin respuesta"
            channel = "Telegram" if item["source"] == "telegram" else "App"
            when = local_time_label(item["timestamp"], self.cfg["timezone"])
            lines.append(f"{index}) {request_id} · {channel} · {when}")
            lines.append(f"   Mensaje: «{one_line(item['content'], 160) or '(vacío)'}»")
            if usage is None:
                lines.append("   Tokens: sin datos")
            elif usage["calls"] == 0:
                lines.append("   Tokens: sin consumo")
            else:
                lines.append(f"   Tokens: prompt={usage['prompt']} "
                             f"completion={usage['completion']} "
                             f"total={usage['total']} llamadas={usage['calls']}")
            lines.append(f"   Resultado: {result}")
            lines.append("")
        totals = self.token_usage
        lines.extend([
            "Consumo total",
            f"Tokens: prompt={totals['prompt']} completion={totals['completion']} "
            f"total={totals['total']}",
            f"Llamadas: {totals['calls']}",
        ])
        for model, usage in totals["by_model"].items():
            lines.append(f"Modelo: {model} · total={usage['total']} "
                         f"llamadas={usage['calls']}")
        active = self.sessions.get(self.active_id) if self.active_id else None
        if active is None:
            lines.append("Sesión activa: nueva (se creará con el próximo mensaje)")
        else:
            lines.append(f"Sesión activa: {active['id']} ({active['state']})")
        lines.append(f"Cola: {self.jobs.qsize()} · En vuelo: {len(self.inflight)}")
        return "\n".join(lines)

    def _local_reply(self, text: str, source: str, chat_id: int | None = None) -> dict:
        """Respuesta directa sin Groq (comandos). Ahorra cuota y es exacta."""
        command_text = text.strip()
        command = COMMAND_ALIASES.get(command_text.split(maxsplit=1)[0].casefold())
        dlog(self, "LOCAL_REPLY", f"command={command_text} source={source}")
        with self.lock:
            if command == "clear":
                # Sesión nueva y vacía: contexto realmente limpio.
                self.active_id = None
            session = self._active()
            session["state"] = "completed"
            session["updated_at"] = utc_now()
            request_id = new_id("req")
            if command == "clear":
                content = "Historial reiniciado. ¿En qué le puedo ayudar?"
            elif command == "log":
                content = self._log_text()
            else:  # /help y comandos desconocidos
                unknown = None if command == "help" else command_text
                content = self._help_text(unknown)
            self._remember(request_id, usage={"prompt": 0, "completion": 0,
                                              "total": 0, "calls": 0},
                           result="local")
            if command != "clear":
                session["messages"].append({
                    "requestId": request_id, "sessionId": session["id"],
                    "role": "user", "content": text, "source": source,
                    "timestamp": utc_now(),
                })
            # Mismo requestId que el evento: la hidratación sustituye el
            # historial y la UI no debe perder la identidad del mensaje.
            session["messages"].append({
                "requestId": request_id, "sessionId": session["id"],
                "role": "assistant", "content": content, "source": "local",
                "timestamp": utc_now(),
            })
            session["messages"] = session["messages"][-STORE_HISTORY:]
            self._save_sessions()
            self._push_state(session["id"], request_id, "thinking")
            if command != "clear":
                self._push_message(session["id"], request_id, "user", text, source)
            self._push_state(session["id"], request_id, "responding")
            self._push_message(session["id"], request_id, "assistant",
                               content, "local")
            self._push_state(session["id"], request_id, "completed")
            if source == "telegram" and chat_id is not None:
                self._send_telegram(chat_id, content)
            return {"requestId": request_id, "sessionId": session["id"]}

    def receive(self, content: str, source: str, chat_id: int | None = None) -> dict:
        """Registra una petición y la encola al worker. Devuelve ids."""
        dlog(self, "RECEIVE", f"source={source} chat_id={chat_id} content={content[:200]}")
        text = content.strip()
        if not text:
            raise ValueError("empty_message")
        text = text[:MAX_CONTENT_LEN]
        if text.startswith("/"):
            # Comandos locales (conocidos o no): nunca gastan una llamada a Groq.
            return self._local_reply(text, source, chat_id)
        with self.lock:
            session = self._active()
            session["state"] = "thinking"
            session["updated_at"] = utc_now()
            request_id = new_id("req")
            session["messages"].append({
                "requestId": request_id, "sessionId": session["id"],
                "role": "user", "content": text, "source": source,
                "timestamp": utc_now(),
            })
            session["messages"] = session["messages"][-STORE_HISTORY:]
            self._save_sessions()
            self._push_state(session["id"], request_id, "thinking")
            self._push_message(session["id"], request_id, "user", text, source)
            self.jobs.put({"request_id": request_id, "session_id": session["id"],
                           "source": source, "chat_id": chat_id,
                           "deadline": time.monotonic() + OP_TIMEOUT_S})
            self.inflight.add(request_id)
            return {"requestId": request_id, "sessionId": session["id"]}

    def cancel(self, request_id: str) -> bool:
        """Marca cancelada. Idempotente: desconocida también devuelve ok."""
        dlog(self, "CANCEL", f"request={request_id}")
        with self.lock:
            # El conjunto se poda: solo importan las cancelaciones recientes
            # (mejor esfuerzo; una cancelación tardía se ignora igual).
            if len(self.cancelled) > 2000:
                self.cancelled.clear()
            self.cancelled.add(request_id)
            found = False
            for session in self.sessions.values():
                pending_here = any(
                    m.get("requestId") == request_id and m.get("role") == "user"
                    for m in session["messages"][-5:]
                )
                if pending_here and session["state"] not in TERMINAL_STATES:
                    session["state"] = "cancelled"
                    session["updated_at"] = utc_now()
                    self._push_state(session["id"], request_id, "cancelled")
                    self._save_sessions()
                    found = True
            # Interrumpe una confirmación en espera (equivale a denegar).
            entry = self.confirmations.pop(request_id, None)
            if entry is not None:
                entry["decision"] = "deny"
                entry["event"].set()
            removed = [iid for iid, entry in self._delegations.items()
                       if entry.get("request_id") == request_id]
            if removed:
                for iid in removed:
                    self._delegations.pop(iid, None)
                self._save_delegations()
            return found

    def cancel_all(self) -> int:
        """Cancela todas las peticiones en vuelo (cola + activa + confirmaciones)."""
        with self.lock:
            ids = list(self.inflight)
        for request_id in ids:
            self.cancel(request_id)  # cancel() toma su propio lock
        dlog(self, "CANCEL", f"cancel_all count={len(ids)}")
        return len(ids)

    # -- Groq ------------------------------------------------------------
    def _context_block(self, channel: str) -> str:
        """Fecha/hora server-side (zona configurable) + canal. Bloque variable,
        separado del sistema fijo para no romper la caché de prefijo."""
        now = datetime.now(zoneinfo.ZoneInfo(self.cfg["timezone"]))
        canal = "Telegram" if channel == "telegram" else "aplicación"
        return (f"Contexto actual: {_DIAS[now.weekday()]} {now.day} de "
                f"{_MESES[now.month - 1]} de {now.year}, "
                f"{now.strftime('%H:%M')} ({self.cfg['timezone']}). "
                f"Canal actual: {canal}.")

    # -- herramientas ----------------------------------------------------
    def tool_defs(self) -> list[dict]:
        if self._hefesto_journal is not None and self._hefesto_device is not None:
            return TOOL_DEFS + [HEFESTO_TOOL]
        return TOOL_DEFS

    def _tool_def(self, tool_id: str) -> dict | None:
        return next((t for t in self.tool_defs() if t["id"] == tool_id), None)

    def _resolve_calendar(self, name: str) -> dict | None:
        """Calendario por nombre (insensible a mayúsculas/espacios) o por id."""
        if not name:
            return None
        try:
            client = self._caldav_client()
        except atlas_caldav.CalDAVError:
            return None
        wanted = norm_name(name)
        for cal in client.calendars:
            if norm_name(cal.get("name", "")) == wanted or cal.get("id") == name:
                return cal
        return None

    def execute_tool(self, tool_id: str, args: dict, job: dict) -> str:
        """Ejecuta una herramienta. Devuelve texto para el modelo."""
        if tool_id == "hefesto_echo":
            return self._delegate_echo(job, args)
        if tool_id == "calendar_read":
            try:
                days = max(1, min(int(args.get("days", 7)), 31))
            except (ValueError, TypeError):
                days = 7
            try:
                events = self.calendar_upcoming(days)
            except atlas_caldav.CalDAVError as exc:
                return f"calendario no disponible ({exc})"
            wanted = args.get("calendar")
            if wanted:
                events = [e for e in events
                          if norm_name(e.get("calendar", "")) == norm_name(str(wanted))]
            if not events:
                return "sin eventos en ese rango"
            lines = [f"{e['start'][:16]} {e['summary'][:80]} [{e['calendar']}] "
                     f"(uid {e['uid']})" for e in events[:20]]
            return "Eventos:\n" + "\n".join(lines)
        if tool_id == "calendar_write":
            return self._execute_write(args)
        return "herramienta desconocida"

    def _delegate_echo(self, job: dict, args: dict) -> str:
        journal, device_id = self._hefesto_journal, self._hefesto_device
        if journal is None or device_id is None:
            return "Hefesto no está disponible (desactivado)"
        text = str(args.get("text", "")).strip()
        if not text:
            return "delegación rechazada: texto vacío"
        if len(text.encode("utf-8")) > 256:
            return "delegación rechazada: texto demasiado largo (máx. 256 bytes)"
        submission = {
            "protocolVersion": 1,
            "submissionId": new_id("sub"),
            "requestId": job["request_id"],
            "sessionId": job["session_id"],
            "taskId": new_id("tsk"),
            "toolId": "hefesto.echo",
            "arguments": {"text": text},
        }
        status, response = journal.dispatch(
            hefesto.Peer("controller", device_id), "POST",
            "/v1/hefesto/submit", json.dumps(submission).encode("utf-8"))
        if status != 200:
            dlog(self, "HEFESTO",
                 f"submit rejected error={response.get('error')} request={job['request_id']}")
            return f"Hefesto no está disponible ({response.get('error', 'desconocido')})"
        invocation_id = response["invocationId"]
        with self.lock:
            if job["request_id"] in self.cancelled:
                return "cancelada a mitad de turno"
            self._delegations[invocation_id] = {
                "invocation_id": invocation_id,
                "request_id": job["request_id"],
                "session_id": job["session_id"],
                "task_id": submission["taskId"],
                "source": job.get("source", "local"),
                "chat_id": job.get("chat_id"),
            }
            self._save_delegations()
        dlog(self, "HEFESTO", f"delegated invocation={invocation_id} "
                              f"request={job['request_id']} task={submission['taskId']}")
        return f"delegado a Hefesto ({invocation_id})"

    def _hefesto_finish(self, entry: dict, state: str,
                        output_text: str | None) -> None:
        if state == "succeeded" and output_text is not None:
            content = output_text
        elif state == "expired":
            content = "Hefesto no respondió a tiempo y el encargo caducó."
        elif state == "cancelled":
            content = "El encargo a Hefesto se canceló."
        else:
            content = "El encargo a Hefesto quedó en un estado desconocido."
        with self.lock:
            session = self.sessions.get(entry["session_id"])
            if session is None or session["id"] != self.active_id:
                self._delegations.pop(entry["invocation_id"], None)
                self._save_delegations()
                return
            request_id = new_id("req")
            session["messages"].append({
                "requestId": request_id, "sessionId": session["id"],
                "role": "assistant", "content": content, "source": "local",
                "agent": "hefesto", "taskId": entry["task_id"],
                "timestamp": utc_now(),
            })
            session["messages"] = session["messages"][-STORE_HISTORY:]
            self._save_sessions()
            self._delegations.pop(entry["invocation_id"], None)
            self._save_delegations()
            self._push_event({
                "kind": "message", "type": "conversation.message_appended",
                "requestId": request_id, "sessionId": session["id"],
                "timestamp": utc_now(), "role": "assistant", "content": content,
                "source": "local", "agent": "hefesto",
                "taskId": entry["task_id"],
            })
        if entry.get("chat_id") is not None:
            self._send_telegram(entry["chat_id"],
                                f"Hefesto: {content}"[:MAX_CONTENT_LEN])
        dlog(self, "HEFESTO", f"continuation invocation={entry['invocation_id']} "
                              f"state={state} session={entry['session_id']}")

    def hefesto_start(self, journal, device_id: str | None) -> None:
        with self.lock:
            self._hefesto_journal = journal
            self._hefesto_device = device_id
        if device_id is None:
            return
        self._hefesto_stop = threading.Event()
        self._hefesto_thread = threading.Thread(
            target=self._hefesto_tracker, name="hefesto-tracker", daemon=True)
        self._hefesto_thread.start()
        dlog(self, "HEFESTO", f"tracker started device={device_id} "
                              f"delegations={len(self._delegations)}")

    def hefesto_stop(self) -> None:
        stop = self._hefesto_stop
        if stop is not None:
            stop.set()
        thread = self._hefesto_thread
        if thread is not None and thread.ident is not None:
            thread.join(timeout=2)

    def _hefesto_tracker(self) -> None:
        while not self._hefesto_stop.wait(1.0):
            self._hefesto_poll()

    def _hefesto_poll(self) -> None:
        with self.lock:
            journal, device_id = self._hefesto_journal, self._hefesto_device
            entries = [dict(entry) for entry in self._delegations.values()]
        if journal is None or device_id is None or not entries:
            return
        peer = hefesto.Peer("controller", device_id)
        for entry in entries:
            status, response = journal.dispatch(
                peer, "POST", "/v1/hefesto/outcome",
                json.dumps({"protocolVersion": 1,
                            "invocationId": entry["invocation_id"]}).encode("utf-8"))
            if status != 200:
                dlog(self, "HEFESTO",
                     f"outcome error={response.get('error')} "
                     f"invocation={entry['invocation_id']}")
                continue
            if response.get("state") in ("succeeded", "expired", "cancelled", "unknown"):
                self._hefesto_finish(entry, response["state"],
                                     response.get("outputText"))

    def _execute_write(self, args: dict) -> str:
        action = args.get("action")
        calendar_name = str(args.get("calendar", ""))
        if action not in ("create", "update", "delete"):
            return "acción inválida (create|update|delete)"
        # Rechazo rápido sin red cuando viene un nombre (los ids con "/" pasan
        # a resolución; tras descubrir se revalida igualmente).
        if "/" not in calendar_name and norm_name(calendar_name) not in self.cfg["editable"]:
            return "calendario no editable (solo lectura)"
        try:
            client = self._caldav_client()
        except atlas_caldav.CalDAVError as exc:
            return f"calendario no disponible ({exc})"
        wanted = norm_name(calendar_name)
        cal = next((c for c in client.calendars
                    if norm_name(c.get("name", "")) == wanted
                    or c.get("id") == calendar_name), None)
        if cal is None:
            return "calendario desconocido"
        if norm_name(cal["name"]) not in self.cfg["editable"]:
            return "calendario no editable (solo lectura)"
        try:
            if action == "create":
                summary = str(args.get("summary", "")).strip()
                start = str(args.get("start", "")).strip()
                if not summary or not start:
                    return "create necesita summary y start"
                uid = new_id("evt").replace("evt-", "") + "@atlas"
                ics = atlas_caldav.build_vevent(
                    uid, summary, start,
                    str(args.get("end", "") or "") or None,
                    str(args.get("description", "") or ""),
                    str(args.get("location", "") or ""),
                    self.cfg["timezone"])
                url = client.collection_url(cal["id"]) + uid + ".ics"
                client.put_event(url, ics, new=True)
                return f"creado «{summary}» en {cal['name']} (uid {uid})"
            uid = str(args.get("uid", "")).strip()
            if not uid:
                return f"{action} necesita uid"
            url = client.find_event_url(cal["id"], uid)
            if not url:
                return "evento no encontrado"
            if action == "delete":
                client.delete_event(url)
                return f"borrado en {cal['name']} (uid {uid})"
            raw = client.get_event(url)
            patched = atlas_caldav.patch_vevent(
                raw,
                str(args.get("summary") or "") or None,
                str(args.get("start") or "") or None,
                str(args.get("end") or "") or None,
                str(args.get("description") or "") or None,
                str(args.get("location") or "") or None,
                self.cfg["timezone"])
            if patched is None:
                return "evento ilegible"
            client.put_event(url, patched)
            return f"modificado en {cal['name']} (uid {uid})"
        except atlas_caldav.CalDAVError as exc:
            return f"calendario: {exc}"
        except ValueError:
            return "fecha inválida (YYYY-MM-DD o YYYY-MM-DDTHH:MM)"

    def _confirm_summary(self, tool_id: str, args: dict) -> tuple[str, str]:
        """(resumen para la UI, riesgo)."""
        if tool_id == "calendar_write":
            action = args.get("action", "?")
            calendar = args.get("calendar", "?")
            summary = str(args.get("summary", ""))
            start = str(args.get("start", ""))
            uid = str(args.get("uid", ""))[:8]
            if action == "create":
                text = f"Crear «{summary}» en {calendar}" + (f" el {start}" if start else "")
            elif action == "update":
                text = f"Modificar evento ({uid}…) en {calendar}"
                if summary:
                    text += f": «{summary}»"
            elif action == "delete":
                text = f"Borrar evento ({uid}…) en {calendar}"
            else:
                text = f"Calendario: {action} en {calendar}"
            return (text, "write")
        return (f"Ejecutar {tool_id}", "external")

    def ask_confirmation(self, request_id: str, session_id: str,
                         tool_id: str, args: dict,
                         chat_id: int | None = None) -> str | None:
        """Emite la petición y espera. Devuelve la decisión o None (timeout)."""
        summary, risk = self._confirm_summary(tool_id, args)
        entry = {"tool": tool_id, "args": args, "event": threading.Event(),
                 "decision": None, "expires": time.time() + CONFIRM_TIMEOUT_S,
                 "chat_id": chat_id,
                 "session_id": session_id}
        with self.lock:
            self.confirmations[request_id] = entry
            self._push_event({
                "kind": "confirmation", "type": "permission.confirmation_required",
                "requestId": request_id, "sessionId": session_id,
                "timestamp": utc_now(), "toolId": tool_id,
                "summary": summary, "risk": risk,
                "expiresAt": time.strftime("%Y-%m-%dT%H:%M:%SZ",
                                           time.gmtime(entry["expires"])),
            })
        # Avisar por Telegram si hay chat asociado (fuera del lock por red).
        if chat_id is not None:
            dlog(self, "CONFIRM", f"enviando aviso a Telegram chat_id={chat_id} summary={summary[:100]}")
            self._send_telegram(
                chat_id,
                f"⏳ Atlas necesita tu confirmación:\n{summary}\n\n"
                "Responde «sí» / «adelante» para aceptar, o «no» para denegar.\n"
                "Tienes 5 minutos para responder.")
        else:
            dlog(self, "CONFIRM", f"sin chat_id (local), no se envía aviso Telegram request={request_id}")
        entry["event"].wait(CONFIRM_TIMEOUT_S)
        with self.lock:
            self.confirmations.pop(request_id, None)
        return entry["decision"]

    def find_pending_confirmation_for_chat(self, chat_id: int) -> str | None:
        """Busca una confirmación pendiente asociada a un chat de Telegram."""
        now = time.time()
        with self.lock:
            for request_id, entry in self.confirmations.items():
                if entry.get("chat_id") == chat_id and now < entry["expires"]:
                    return request_id
        return None

    def confirm(self, request_id: str, decision: str) -> bool:
        """Registra la decisión del usuario. False si caducó o no existe."""
        dlog(self, "CONFIRM", f"request={request_id} decision={decision}")
        with self.lock:
            entry = self.confirmations.get(request_id)
            if entry is None or time.time() > entry["expires"]:
                self.confirmations.pop(request_id, None)
                return False
            session_id = entry.get("session_id", "")
            chat_id = entry.get("chat_id")
            entry["decision"] = decision
            entry["event"].set()
            # Empujar evento para que el frontend sepa que se resolvió.
            self._push_event({
                "kind": "confirmation_resolved",
                "type": "permission.resolved",
                "requestId": request_id,
                "sessionId": session_id,
                "timestamp": utc_now(),
                "decision": decision,
            })
            # Avisar por Telegram si hay chat asociado.
            if chat_id is not None:
                icon = "✅" if decision == "allow_once" else "❌"
                verb = "aceptada" if decision == "allow_once" else "denegada"
                self._send_telegram(chat_id, f"{icon} Acción {verb}.")
            return True

    def _account_usage(self, raw: dict) -> dict:
        """Acumula el consumo de un turno y devuelve el delta normalizado."""
        try:
            prompt = int(raw.get("prompt_tokens", 0) or 0)
            completion = int(raw.get("completion_tokens", 0) or 0)
            total = int(raw.get("total_tokens", 0) or 0)
        except (ValueError, TypeError):
            prompt = completion = total = 0
        model = self.cfg["groq_model"]
        with self.lock:
            self.token_usage["prompt"] += prompt
            self.token_usage["completion"] += completion
            self.token_usage["total"] += total
            self.token_usage["calls"] += 1
            bucket = self.token_usage["by_model"].setdefault(
                model, {"prompt": 0, "completion": 0, "total": 0, "calls": 0})
            bucket["prompt"] += prompt
            bucket["completion"] += completion
            bucket["total"] += total
            bucket["calls"] += 1
        return {"prompt": prompt, "completion": completion,
                "total": total, "calls": 1}

    def groq_completion(self, messages: list[dict]
                        ) -> tuple[str | None, list | None, str | None, dict | None]:
        """Un turno con tools. Devuelve (contenido, tool_calls, error, uso)."""
        keys = self.cfg["groq_keys"]
        if not keys:
            return (None, None, "no_keys", None)
        body = json.dumps({
            "model": self.cfg["groq_model"],
            "temperature": self.cfg["groq_temperature"],
            "messages": messages,
            "tools": tool_payload(self.tool_defs()),
            "tool_choice": "auto",
        }).encode("utf-8")
        last_err = "unknown"
        for index, key in enumerate(keys):
            req = urllib.request.Request(
                GROQ_API, data=body,
                headers={"Authorization": f"Bearer {key}",
                         "Content-Type": "application/json",
                         "User-Agent": USER_AGENT},
                method="POST",
            )
            try:
                with urllib.request.urlopen(req, timeout=GROQ_TIMEOUT_S) as response:
                    data = json.load(response)
                delta = self._account_usage(data.get("usage") or {})
                dlog(self, "LLM", f"groq completion ok model={self.cfg['groq_model']}",
                     prompt_tokens=delta["prompt"],
                     completion_tokens=delta["completion"],
                     total_tokens=delta["total"])
                message = (data.get("choices") or [{}])[0].get("message", {})
                content = (message.get("content") or "").strip() or None
                calls = message.get("tool_calls") or None
                if content is None and not calls:
                    return (None, None, "empty_response", delta)
                return (content, calls, None, delta)
            except urllib.error.HTTPError as exc:
                if exc.code in (401, 403, 429) or 500 <= exc.code < 600:
                    last_err = f"http_{exc.code}"
                    if index + 1 < len(keys):
                        continue
                return (None, None, f"http_{exc.code}", None)
            except Exception:
                last_err = "unreachable"
                if index + 1 < len(keys):
                    continue
                return (None, None, "unreachable", None)
        return (None, None, last_err, None)

    # -- worker ----------------------------------------------------------
    def worker(self) -> None:
        while True:
            job = self.jobs.get()
            try:
                self._process(job)
            except Exception as exc:  # el worker nunca muere
                log(f"job falló ({type(exc).__name__}), sigue.")
            finally:
                self.jobs.task_done()

    def _complete(self, request_id: str, job: dict, text: str,
                  error: str | None, usage: dict | None = None) -> None:
        """Cierra la operación (vía común de stub, LLM y tools)."""
        dlog(self, "COMPLETE", f"request={request_id} error={error} text_len={len(text or '')}")
        with self.lock:
            self.inflight.discard(request_id)
            self._remember(request_id, usage=usage)
            session = self.sessions.get(job["session_id"])
            if session is None:
                self._remember(request_id, result="sesión perdida")
                return
            if request_id in self.cancelled:
                self.cancelled.discard(request_id)
                self._remember(request_id, result="cancelada")
                return  # cancelada a mitad: sin efectos
            if time.monotonic() > job["deadline"]:
                error, text = "timeout", ""
            self._remember(request_id,
                           result="completada" if error is None else f"error ({error})")
            session["updated_at"] = utc_now()
            if error is None:
                session["state"] = "completed"
                # Correlación estable con el evento y el mensaje del usuario:
                # si el id cambiara, la re-hidratación del frontal reiniciaría
                # la impresión del mensaje a mitad.
                session["messages"].append({
                    "requestId": request_id, "sessionId": session["id"],
                    "role": "assistant", "content": text, "source": "local",
                    "timestamp": utc_now(),
                })
                session["messages"] = session["messages"][-STORE_HISTORY:]
                self._save_sessions()
                self._push_state(session["id"], request_id, "responding")
                self._push_message(session["id"], request_id, "assistant", text, "local")
                self._push_state(session["id"], request_id, "completed")
            else:
                session["state"] = "failed"
                self._save_sessions()
                self._push_state(session["id"], request_id, "failed", error)
            if job["source"] == "telegram" and job.get("chat_id") is not None:
                reply = text if error is None else "Lo siento, ahora mismo no puedo responder."
                self._send_telegram(job["chat_id"], reply[:MAX_CONTENT_LEN])

    def _handle_tool_call(self, job: dict, call: dict) -> str:
        """Ejecuta una tool_call del modelo (con confirmación si toca)."""
        function = call.get("function", {}) if isinstance(call, dict) else {}
        dlog(self, "TOOL", f"call tool={function.get('name')} args={function.get('arguments', '')[:300]} request={job.get('request_id')}")
        try:
            function = call["function"]
            name = function["name"]
            args = json.loads(function.get("arguments") or "{}")
            if not isinstance(args, dict):
                args = {}
        except (ValueError, KeyError, TypeError, AttributeError):
            return "llamada malformada"
        tool = self._tool_def(name)
        if tool is None:
            return "herramienta desconocida"
        if job.get("request_id") in self.cancelled:
            return "cancelada a mitad de turno"
        if tool["requiresConfirmation"]:
            # El tiempo del usuario no consume el plazo de la operación.
            job["deadline"] = time.monotonic() + CONFIRM_TIMEOUT_S + 60
            decision = self.ask_confirmation(job["request_id"], job["session_id"],
                                             name, args, chat_id=job.get("chat_id"))
            if decision != "allow_once":
                return ("denegada por el usuario"
                        if decision == "deny" else "caducada sin respuesta")
        return self.execute_tool(name, args, job)

    def _process(self, job: dict) -> None:
        request_id = job["request_id"]
        session_id = job["session_id"]
        channel = job.get("source", "local")
        dlog(self, "PROCESS", f"start request={request_id} session={session_id} channel={channel}")
        with self.lock:
            session = self.sessions.get(session_id)
            if session is None:
                dlog(self, "PROCESS", f"session not found, aborting request={request_id}")
                return
            history = [{"role": m["role"], "content": m["content"]}
                       for m in session["messages"]]
        if not self.cfg["groq_keys"]:
            last_user = next((m["content"] for m in reversed(history)
                              if m["role"] == "user"), "")
            dlog(self, "PROCESS", f"no groq keys, stub reply request={request_id}")
            self._complete(request_id, job,
                           f"(stub local · sin claves Groq) Recibido: {last_user}",
                           None, usage={"prompt": 0, "completion": 0,
                                        "total": 0, "calls": 0})
            return
        messages: list[dict] = [
            {"role": "system", "content": SYSTEM_FIXED},
            {"role": "system", "content": self._context_block(channel)},
        ]
        messages.extend(history[-MAX_HISTORY:])
        usage_total = {"prompt": 0, "completion": 0, "total": 0, "calls": 0}
        for turn in range(MAX_TURNS):
            dlog(self, "PROCESS", f"turn {turn+1}/{MAX_TURNS} request={request_id}")
            content, calls, error, usage = self.groq_completion(messages)
            if usage is not None:
                for key in usage_total:
                    usage_total[key] += usage[key]
            if error is not None:
                dlog(self, "PROCESS", f"groq error={error} request={request_id}")
                self._complete(request_id, job, "", error, usage_total)
                return
            if not calls:
                dlog(self, "PROCESS", f"final response len={len(content or '')} request={request_id}")
                self._complete(request_id, job, content or "(vacío)", None, usage_total)
                return
            tool_names = [c.get("function", {}).get("name") for c in (calls or [])]
            dlog(self, "PROCESS", f"tool_calls={tool_names} request={request_id}")
            messages.append({"role": "assistant", "content": content or "",
                             "tool_calls": calls})
            for call in calls:
                result = self._handle_tool_call(job, call if isinstance(call, dict) else {})
                call_id = call.get("id", "") if isinstance(call, dict) else ""
                dlog(self, "TOOL", f"tool={call.get('function', {}).get('name')} result={result[:200]} request={request_id}")
                messages.append({"role": "tool", "tool_call_id": call_id,
                                 "content": result})
        dlog(self, "PROCESS", f"max turns reached request={request_id}")
        self._complete(request_id, job, "", "max_turns", usage_total)

    # -- Telegram --------------------------------------------------------
    def _telegram_call(self, method: str, payload: dict, timeout: int) -> dict:
        data = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(
            f"{TELEGRAM_API}/bot{self.cfg['token']}/{method}", data=data,
            headers={"Content-Type": "application/json",
                     "User-Agent": USER_AGENT},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=timeout) as response:
            return json.load(response)

    def _send_telegram(self, chat_id: int, text: str) -> None:
        dlog(self, "TELEGRAM", f"send chat_id={chat_id} text={text[:200]}")
        try:
            self._telegram_call("sendMessage", {"chat_id": chat_id, "text": text}, 20)
        except Exception as exc:
            log(f"sendMessage falló ({type(exc).__name__}).")

    def telegram_loop(self) -> None:
        token = self.cfg["token"]
        try:
            me = self._telegram_call("getMe", {}, 20)
            result = me.get("result") or {}
            if result.get("username"):
                with self.lock:
                    self.bot_info = {"id": result.get("id"),
                                     "username": result.get("username"),
                                     "first_name": result.get("first_name")}
        except urllib.error.HTTPError as exc:
            if exc.code in (401, 404):
                # SystemExit en un hilo no detiene el proceso: salida dura.
                log("token de Telegram inválido (getMe 401/404). Relay detenido.")
                os._exit(2)
            log(f"getMe falló ({exc.code}), reintentando.")
        except OSError as exc:
            log(f"sin red para getMe ({exc}), reintentando.")
        log(f"Telegram activo (offset {self.offset}, "
            f"{len(self.cfg['allowed'])} id(s) autorizados).")
        fails = 0
        while True:
            try:
                data = self._telegram_call(
                    "getUpdates",
                    {"offset": self.offset, "timeout": TELEGRAM_POLL_TIMEOUT_S,
                     "allowed_updates": ["message"]},
                    TELEGRAM_POLL_TIMEOUT_S + 10,
                )
            except Exception as exc:  # red o Telegram caídos: backoff y seguir
                fails += 1
                wait = min(60, TELEGRAM_RETRY_S * (2 ** min(fails, 4)))
                log(f"poll falló ({type(exc).__name__}), reintento en {wait}s.")
                time.sleep(wait)
                continue
            fails = 0
            for update in data.get("result", []):
                self.offset = max(self.offset, int(update.get("update_id", 0)) + 1)
                message = update.get("message", {})
                sender = message.get("from", {}).get("id")
                chat_id = message.get("chat", {}).get("id")
                text = (message.get("text") or message.get("caption") or "").strip()
                if not text or sender not in self.cfg["allowed"] or chat_id is None:
                    continue
                text_lower = text.lower().strip()
                # 1. ¿Es una confirmación por texto para una operación pendiente?
                pending = self.find_pending_confirmation_for_chat(chat_id)
                if pending is not None:
                    if text_lower in CONFIRM_WORDS:
                        if self.confirm(pending, "allow_once"):
                            self._send_telegram(
                                chat_id,
                                "✅ Confirmado. Ejecutando la acción…")
                        else:
                            self._send_telegram(
                                chat_id,
                                "❌ La confirmación ya caducó. Pide de nuevo la acción.")
                        continue
                    if text_lower in DENY_WORDS:
                        if self.confirm(pending, "deny"):
                            self._send_telegram(
                                chat_id,
                                "❌ Denegado. Acción cancelada.")
                        else:
                            self._send_telegram(
                                chat_id,
                                "⚠️ No había confirmación pendiente para denegar.")
                        continue
                    # Texto no reconocido: pedir claridad sin crear nueva petición
                    self._send_telegram(
                        chat_id,
                        "Hay una acción pendiente esperando tu confirmación. "
                        "Responde «sí» / «adelante» para aceptar o «no» para denegar.")
                    continue
                # 2. No hay confirmación pendiente: procesar como petición nueva.
                try:
                    self.receive(text, "telegram", chat_id)
                except ValueError:
                    pass
            self._save_offset()


def make_handler(brain: Brain, network: ipaddress.IPv4Network, status: dict,
                 sysmon: system_monitor.SystemMonitor):
    # RELAY_CORS_ORIGIN Puede llevar varios orígenes separados por comas
    # (dev 5173 del app + dev 5174 del panel): se devuelve SOLO el Origin que
    # hace petición si está en la lista (formato válido de CORS multi-origen).
    allowed_origins = [o.strip() for o in brain.cfg["cors_origin"].split(",")
                       if o.strip()]

    class Handler(BaseHTTPRequestHandler):
        server_version = "AtlasRelay/3"
        # HTTP/1.1 mantiene la conexión (keep-alive): el panel sondea varias
        # rutas cada 2 s y así no paga un handshake nuevo en cada petición.
        # El timeout cierra las conexiones ociosas para no dejar hilos colgados.
        protocol_version = "HTTP/1.1"
        timeout = 15

        def _cors(self) -> None:
            origin = self.headers.get("Origin", "")
            if origin and origin in allowed_origins:
                self.send_header("Access-Control-Allow-Origin", origin)

        _CONTENT_TYPES = {".html": "text/html", ".js": "text/javascript",
                          ".css": "text/css", ".svg": "image/svg+xml",
                          ".png": "image/png", ".ico": "image/x-icon",
                          ".json": "application/json", ".map": "application/json",
                          ".woff2": "font/woff2", ".txt": "text/plain"}

        def log_message(self, *args: object) -> None:
            pass  # Los accesos no se registran (menos ruido, sin IPs en logs).

        def _lan_only(self) -> bool:
            try:
                client = ipaddress.ip_address(self.client_address[0])
            except ValueError:
                return False
            if client.is_loopback:
                return True
            return isinstance(client, ipaddress.IPv4Address) and client in network

        def _json(self, status_code: int, payload: dict) -> None:
            body = json.dumps(payload).encode("utf-8")
            self.send_response(status_code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self._cors()
            self.end_headers()
            self.wfile.write(body)

        def _panel_file(self, relpath: str) -> None:
            """Sirve los estáticos del panel (ruta dentro de PANEL_DIST_DIR)."""
            relpath = relpath.split("?", 1)[0].split("#", 1)[0]
            target = os.path.realpath(os.path.join(sysmon.panel_dist, relpath))
            if target == sysmon.panel_dist or not os.path.isdir(sysmon.panel_dist):
                # /panel/ (raíz) o ruta fuera de la raíz → index.html o 404.
                if relpath != "":
                    self._json(404, {"ok": False, "error": "not_found"})
                    return
                target = os.path.join(sysmon.panel_dist, "index.html")
            if not os.path.isfile(target):
                self._json(404, {"ok": False, "error": "not_found"})
                return
            try:
                with open(target, "rb") as fh:
                    body = fh.read()
            except OSError:
                self._json(500, {"ok": False, "error": "panel_read_failed"})
                return
            ext = os.path.splitext(target)[1].lower()
            self.send_response(200)
            self.send_header("Content-Type",
                             self._CONTENT_TYPES.get(ext, "application/octet-stream"))
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-cache")
            self._cors()
            self.end_headers()
            self.wfile.write(body)

        def _read_json(self) -> dict | None:
            try:
                length = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                return None
            if length <= 0 or length > MAX_BODY_BYTES:
                return None
            try:
                return json.loads(self.rfile.read(length))
            except ValueError:
                return None

        def do_OPTIONS(self) -> None:  # noqa: N802 (firma de BaseHTTPRequestHandler)
            if not self._lan_only():
                self._json(403, {"ok": False, "error": "same_network_only"})
                return
            self.send_response(204)
            self._cors()
            if self.headers.get("Origin", "") in allowed_origins:
                self.send_header("Access-Control-Allow-Methods", "GET, POST")
                self.send_header("Access-Control-Allow-Headers", "Content-Type")
            self.end_headers()

        def do_GET(self) -> None:  # noqa: N802 (firma de BaseHTTPRequestHandler)
            if not self._lan_only():
                self._json(403, {"ok": False, "error": "same_network_only"})
                return
            parsed = urllib.parse.urlparse(self.path)
            if parsed.path == "/panel" or parsed.path.startswith("/panel/"):
                self._panel_file(parsed.path[len("/panel/"):])
                return
            if parsed.path == "/v1/health":
                with brain.lock:
                    queued = len(brain.events)
                    inflight = len(brain.inflight)
                bot = dict(brain.bot_info) if brain.bot_info else {}
                self._json(200, {"ok": True, "telegram": status["telegram"],
                                 "llm": status["llm"], "calendar": status["calendar"],
                                 "events": queued, "queue": brain.jobs.qsize(),
                                 "inflight": inflight, "bot": bot})
                return
            if parsed.path == "/v1/system":
                self._json(200, sysmon.snapshot())
                return
            if parsed.path == "/v1/session":
                self._json(200, {"session": brain.get_snapshot(),
                                 "cursor": brain.current_seq()})
                return
            if parsed.path == "/v1/calendar/calendars":
                try:
                    calendars = brain.calendar_calendars()
                except atlas_caldav.CalDAVError as exc:
                    if str(exc) == "no configurado":
                        self._json(501, {"ok": False, "error": "calendar_not_configured"})
                    else:
                        self._json(502, {"ok": False, "error": "calendar_upstream"})
                    return
                self._json(200, {"calendars": calendars})
                return
            if parsed.path == "/v1/calendar/upcoming":
                query = urllib.parse.parse_qs(parsed.query)
                try:
                    days = int(query.get("days", ["7"])[0])
                    if not 1 <= days <= 31:
                        raise ValueError
                except ValueError:
                    self._json(400, {"ok": False, "error": "bad_days"})
                    return
                try:
                    events = brain.calendar_upcoming(days)
                except atlas_caldav.CalDAVError as exc:
                    if str(exc) == "no configurado":
                        self._json(501, {"ok": False, "error": "calendar_not_configured"})
                    else:
                        self._json(502, {"ok": False, "error": "calendar_upstream"})
                    return
                self._json(200, {"events": events})
                return
            if parsed.path == "/v1/events":
                query = urllib.parse.parse_qs(parsed.query)
                try:
                    since = int(query.get("since", ["0"])[0])
                    if since < 0:
                        raise ValueError
                except ValueError:
                    self._json(400, {"ok": False, "error": "bad_since"})
                    return
                self._json(200, {"events": brain.events_since(since)})
                return
            if parsed.path == "/v1/logs":
                query = urllib.parse.parse_qs(parsed.query)
                try:
                    tail = int(query.get("tail", ["100"])[0])
                    if tail < 1 or tail > 10000:
                        raise ValueError
                except ValueError:
                    self._json(400, {"ok": False, "error": "bad_tail"})
                    return
                try:
                    with open(brain.log_path, encoding="utf-8") as fh:
                        lines = fh.readlines()
                except OSError:
                    lines = []
                tail_lines = lines[-tail:]
                self._json(200, {"lines": [ln.rstrip("\\n") for ln in tail_lines],
                                 "total": len(lines)})
                return
            if parsed.path == "/v1/stats":
                with brain.lock:
                    usage = dict(brain.token_usage)
                    usage["by_model"] = {k: dict(v) for k, v in brain.token_usage["by_model"].items()}
                self._json(200, {"token_usage": usage})
                return
            self._json(404, {"ok": False, "error": "not_found"})

        def do_POST(self) -> None:  # noqa: N802 (firma de BaseHTTPRequestHandler)
            if not self._lan_only():
                self._json(403, {"ok": False, "error": "same_network_only"})
                return
            path = urllib.parse.urlparse(self.path).path
            payload = self._read_json()
            if payload is None:
                self._json(400, {"ok": False, "error": "bad_body"})
                return
            if path == "/v1/send":
                content = payload.get("content", "")
                source = payload.get("source", "local")
                if not isinstance(content, str) or not content.strip():
                    self._json(400, {"ok": False, "error": "empty_message"})
                    return
                if source not in ("local", "telegram"):
                    self._json(400, {"ok": False, "error": "bad_source"})
                    return
                try:
                    ids = brain.receive(content, source)
                except ValueError:
                    self._json(400, {"ok": False, "error": "empty_message"})
                    return
                self._json(200, {"ok": True, **ids})
                return
            if path == "/v1/cancel":
                request_id = payload.get("requestId", "")
                if not isinstance(request_id, str) or not request_id:
                    self._json(400, {"ok": False, "error": "bad_request"})
                    return
                cancelled = brain.cancel(request_id)
                self._json(200, {"ok": True, "cancelled": cancelled})
                return
            if path == "/v1/confirm":
                request_id = payload.get("requestId", "")
                decision = payload.get("decision", "")
                if not isinstance(request_id, str) or not request_id:
                    self._json(400, {"ok": False, "error": "bad_request"})
                    return
                if decision not in ("allow_once", "deny"):
                    self._json(400, {"ok": False, "error": "bad_decision"})
                    return
                accepted = brain.confirm(request_id, decision)
                self._json(200, {"ok": True, "accepted": accepted})
                return
            if path == "/v1/system/asks":
                kind = payload.get("kind", "")
                if kind == "service":
                    unit = str(payload.get("unit", ""))
                    verb = str(payload.get("verb", ""))
                    allowed, _scope = sysmon.can_act(unit, verb)
                    if not allowed:
                        self._json(403, {"ok": False, "error": "action_not_allowed"})
                        return
                    detail = {"kind": "service", "unit": unit, "verb": verb,
                              "scope": "system"}
                    summary = f"systemctl {verb} {unit}"
                elif kind == "cancel_all":
                    detail = {"kind": "cancel_all"}
                    summary = "Cancelar TODAS las peticiones en vuelo"
                else:
                    self._json(400, {"ok": False, "error": "bad_kind"})
                    return
                op_id = sysmon.ops.create(kind, summary, detail)
                dlog(brain, "PANEL", f"asks kind={kind} summary={summary}")
                self._json(200, {"ok": True, "op": op_id, "kind": kind,
                                 "summary": summary})
                return
            if path == "/v1/system/resolve":
                op_id = payload.get("op", "")
                decision = payload.get("decision", "")
                if not isinstance(op_id, str) or not op_id:
                    self._json(400, {"ok": False, "error": "bad_request"})
                    return
                if decision not in ("allow_once", "deny"):
                    self._json(400, {"ok": False, "error": "bad_decision"})
                    return
                entry = sysmon.ops.consume(op_id)
                if entry is None:
                    self._json(404, {"ok": False, "error": "expired_or_unknown"})
                    return
                if decision == "deny":
                    dlog(brain, "PANEL", f"deny op={op_id} summary={entry['summary'][:120]}")
                    self._json(200, {"ok": True, "executed": False, "denied": True})
                    return
                detail = entry["detail"]
                if detail["kind"] == "service":
                    result = sysmon.run_service_action(detail["unit"],
                                                       detail["verb"])
                else:  # cancel_all
                    result = {"ok": True, "cancelled": brain.cancel_all()}
                dlog(brain, "PANEL",
                     f"execute kind={detail['kind']} ok={result.get('ok')}")
                self._json(200, {"ok": True, "executed": True, "result": result})
                return
            if path == "/v1/system/cancel_all":
                count = brain.cancel_all()
                self._json(200, {"ok": True, "cancelled": count})
                return
            self._json(404, {"ok": False, "error": "not_found"})

    return Handler


def load_dotenv(path: str = ".env") -> None:
    """Carga `.env` en os.environ si existe. No pisa variables ya definidas.

    Solo stdlib: formato `KEY=VALUE`, comentarios con `#`, sin comillas
    complejas. Cualquier variable ya presente en el entorno (por ejemplo al
    lanzar con `RELAY_PORT=9000 python3 …`) tiene prioridad.
    """
    if not os.path.isfile(path):
        return
    try:
        with open(path, encoding="utf-8") as fh:
            raw = fh.read()
    except OSError:
        return
    for line in raw.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[len("export "):].strip()
        if "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        value = value.strip()
        if not key or key in os.environ:
            continue
        # Quitar comentarios en línea (todo lo que va tras #, salvo que esté
        # entre comillas). Formato estándar de .env.
        if "#" in value and not (len(value) >= 2 and value[0] in "\"'" and value[-1] == value[0]):
            value = value.split("#", 1)[0].strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        os.environ[key] = value


@contextmanager
def hefesto_listener(host: str, data_dir: str):
    names = ("HEFESTO_PORT", "HEFESTO_TLS_CERT", "HEFESTO_TLS_KEY",
             "HEFESTO_TLS_CA", "HEFESTO_PEERS_FILE")
    if not any(name in os.environ for name in names):
        yield None
        return
    if not all(os.environ.get(name, "").strip() for name in names):
        fail_config("Hefesto requiere puerto, certificado, clave, CA y registro de pares.")
    try:
        port = int(os.environ["HEFESTO_PORT"])
        if not 1 <= port <= 65535:
            raise ValueError
    except ValueError:
        fail_config("Puerto de Hefesto inválido.")

    with ExitStack() as resources:
        try:
            peers = hefesto.load_peers(os.environ["HEFESTO_PEERS_FILE"])
            devices = {peer.device_id for peer in peers.values()
                       if peer.role == "device"}
            if len(devices) > 1:
                fail_config("El registro de pares de Hefesto solo admite un dispositivo.")
            device_id = next(iter(devices)) if devices else None
            journal = hefesto.Journal(os.path.join(data_dir, "hefesto.sqlite"))
            resources.callback(journal.close)
            server = hefesto.create_server(
                (host, port), journal, peers,
                certfile=os.environ["HEFESTO_TLS_CERT"],
                keyfile=os.environ["HEFESTO_TLS_KEY"],
                cafile=os.environ["HEFESTO_TLS_CA"],
            )
        except (OSError, ValueError, sqlite3.Error):
            fail_config("Configuración segura de Hefesto no disponible.")
        resources.callback(server.server_close)
        lifecycle = threading.Lock()
        stopping = threading.Event()
        serving = threading.Event()

        def serve():
            with lifecycle:
                if stopping.is_set():
                    return
                serving.set()
            server.serve_forever()

        def stop():
            with lifecycle:
                stopping.set()
                started = serving.is_set()
            if started:
                server.shutdown()
            if thread.ident is not None:
                thread.join()

        thread = threading.Thread(target=serve, name="hefesto-https", daemon=True)
        # También cubre una interrupción dentro de start(), antes de entrar al with.
        resources.callback(stop)
        thread.start()
        log("Hefesto: listener mTLS habilitado (solo eco).")
        yield {"journal": journal, "device_id": device_id}


def main() -> int:
    load_dotenv()
    cfg = read_env()
    os.makedirs(cfg["data_dir"], exist_ok=True)

    if cfg["host"]:
        host = cfg["host"]
    else:
        # Autodetección: si el arranque pilla la red a medias (Wi-Fi/DHCP lento
        # tras un reinicio), `lan_ip()` cae a loopback y el relay quedaría
        # inalcanzable para el panel y el frontal. Reintentamos hasta 60 s y,
        # si sigue sin haber IP LAN, fallamos para que systemd reintente.
        host = lan_ip()
        waited = 0
        while ipaddress.ip_address(host).is_loopback and waited < 60:
            log("sin IP LAN aún; reintentando autodetección en 2 s…")
            time.sleep(2)
            waited += 2
            host = lan_ip()
        if ipaddress.ip_address(host).is_loopback:
            fail_config("no se pudo autodetectar la IP LAN (¿red no lista al arrancar?)")
    try:
        ipaddress.ip_address(host)
    except ValueError:
        fail_config(f"RELAY_HOST no es una IP válida: {host!r}")
    network = ipaddress.ip_network(host + "/24", strict=False)

    brain = Brain(cfg)
    status = {
        "telegram": "enabled" if cfg["token"] else "disabled",
        "llm": "groq" if cfg["groq_keys"] else "stub",
        "calendar": brain.calendar_status(),
    }
    if cfg["token"] and not cfg["allowed"]:
        fail_config("Con bot activo, TELEGRAM_ALLOWED_USER_IDS no puede estar vacío.")
    if not cfg["token"]:
        log("sin TELEGRAM_BOT_TOKEN: Telegram desactivado (canal local + API).")
    log(f"llm: {status['llm']} ({len(cfg['groq_keys'])} claves).")

    sysmon = system_monitor.SystemMonitor(
        log_panel=lambda _kind, message: dlog(brain, "PANEL", message))

    with ThreadingHTTPServer((host, cfg["port"]),
                             make_handler(brain, network, status, sysmon)) as server:
        with hefesto_listener(host, cfg["data_dir"]) as hefesto_ctx:
            if hefesto_ctx is not None:
                brain.hefesto_start(hefesto_ctx["journal"], hefesto_ctx["device_id"])
            threading.Thread(target=brain.worker, daemon=True, name="brain-worker").start()
            if cfg["token"]:
                threading.Thread(target=brain.telegram_loop, daemon=True,
                                 name="telegram-poll").start()
            log(f"escuchando en {host}:{cfg['port']} (solo red local {network}).")
            log(f"panel estático en /panel/ ({sysmon.panel_dist}).")
            try:
                server.serve_forever()
            except KeyboardInterrupt:
                pass
            finally:
                brain.hefesto_stop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
