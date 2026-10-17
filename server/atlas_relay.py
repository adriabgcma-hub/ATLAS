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
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import deque
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import NoReturn

import atlas_caldav

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


def norm_name(name: str) -> str:
    return name.strip().casefold()


def tool_payload() -> list[dict]:
    """Catálogo en formato OpenAI function calling para Groq."""
    return [{"type": "function",
             "function": {"name": t["id"], "description": t["description"],
                          "parameters": t["schema"]}} for t in TOOL_DEFS]


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
        import zoneinfo  # noqa: PLC0415 (stdlib, validación temprana)
        zoneinfo.ZoneInfo(timezone_name)
    except Exception:
        fail_config(f"RELAY_TIMEZONE desconocida: {timezone_name!r}")

    return {
        "token": os.environ.get("TELEGRAM_BOT_TOKEN", "").strip(),
        "allowed": allowed,
        "groq_keys": groq_keys,
        "groq_model": model,
        "groq_temperature": temperature,
        "timezone": timezone_name,
        "caldav_user": os.environ.get("CALDAV_USERNAME", "").strip(),
        "caldav_password": os.environ.get("CALDAV_APP_PASSWORD", "").strip(),
        "editable": {norm_name(chunk) for chunk in
                     os.environ.get("CALDAV_EDIT_CALENDARS",
                                    EDITABLE_DEFAULT).split(",") if chunk.strip()},
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
        # Logging detallado + consumo de tokens (Groq).
        self.log_path = os.path.join(cfg["data_dir"], "relay.log")
        self.token_usage = {"prompt": 0, "completion": 0, "total": 0,
                            "calls": 0, "by_model": {}}
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
    def _local_reply(self, text: str, source: str, chat_id: int | None = None) -> dict:
        """Respuesta directa sin Groq (comandos). Ahorra cuota y es exacta."""
        dlog(self, "LOCAL_REPLY", f"command={text.strip()} source={source}")
        command = text.strip()
        with self.lock:
            if command == "/clear":
                # Sesión nueva y vacía: contexto realmente limpio.
                self.active_id = None
            session = self._active()
            session["state"] = "completed"
            session["updated_at"] = utc_now()
            request_id = new_id("req")
            if command == "/clear":
                content = "Historial reiniciado. ¿En qué le puedo ayudar?"
            elif command == "/help":
                content = ("Comandos: /clear (reiniciar la conversación), "
                           "/log (últimos 3 registros del log). "
                           "También puedo consultar su calendario iCloud "
                           "y, con su permiso, crear, modificar o borrar "
                           "eventos en sus calendarios editables.")
                session["messages"].append({
                    "requestId": request_id, "sessionId": session["id"],
                    "role": "user", "content": text, "source": source,
                    "timestamp": utc_now(),
                })
            else:  # /log
                try:
                    with open(self.log_path, encoding="utf-8") as fh:
                        lines = fh.readlines()
                    last3 = [ln.rstrip("\\n") for ln in lines[-3:]]
                    log_part = "\n".join(last3) if last3 else "Log vacío."
                except OSError:
                    log_part = "No se pudo leer el log."
                # Unificar con consumo de tokens (lectura sin lock para evitar
                # deadlock: ya estamos dentro de self.lock).
                u = self.token_usage
                stats_part = (f"Tokens: prompt={u['prompt']} completion={u['completion']} "
                              f"total={u['total']} llamadas={u['calls']}")
                content = "Últimos 3 registros del log:\n" + log_part + "\n\n" + stats_part
                session["messages"].append({
                    "requestId": request_id, "sessionId": session["id"],
                    "role": "user", "content": text, "source": source,
                    "timestamp": utc_now(),
                })
            session["messages"].append({
                "requestId": new_id("req"), "sessionId": session["id"],
                "role": "assistant", "content": content, "source": "local",
                "timestamp": utc_now(),
            })
            session["messages"] = session["messages"][-STORE_HISTORY:]
            self._save_sessions()
            self._push_state(session["id"], request_id, "thinking")
            if command != "/clear":
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
        if text in ("/clear", "/help", "/log"):
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
            return found

    # -- Groq ------------------------------------------------------------
    def _context_block(self, channel: str) -> str:
        """Fecha/hora server-side (zona configurable) + canal. Bloque variable,
        separado del sistema fijo para no romper la caché de prefijo."""
        import zoneinfo  # noqa: PLC0415 (stdlib)
        now = datetime.now(zoneinfo.ZoneInfo(self.cfg["timezone"]))
        canal = "Telegram" if channel == "telegram" else "aplicación"
        return (f"Contexto actual: {_DIAS[now.weekday()]} {now.day} de "
                f"{_MESES[now.month - 1]} de {now.year}, "
                f"{now.strftime('%H:%M')} ({self.cfg['timezone']}). "
                f"Canal actual: {canal}.")

    # -- herramientas ----------------------------------------------------
    def _tool_def(self, tool_id: str) -> dict | None:
        return next((t for t in TOOL_DEFS if t["id"] == tool_id), None)

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

    def execute_tool(self, tool_id: str, args: dict) -> str:
        """Ejecuta una herramienta. Devuelve texto para el modelo."""
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
            self._send_telegram(
                chat_id,
                f"⏳ Atlas necesita tu confirmación:\n{summary}\n\n"
                "Responde «sí» / «adelante» para aceptar, o «no» para denegar.\n"
                "Tienes 5 minutos para responder.")
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
            entry["decision"] = decision
            entry["event"].set()
            return True

    def groq_completion(self, messages: list[dict]
                        ) -> tuple[str | None, list | None, str | None]:
        """Un turno con tools. Devuelve (contenido, tool_calls, error)."""
        keys = self.cfg["groq_keys"]
        if not keys:
            return (None, None, "no_keys")
        body = json.dumps({
            "model": self.cfg["groq_model"],
            "temperature": self.cfg["groq_temperature"],
            "messages": messages,
            "tools": tool_payload(),
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
                usage = data.get("usage") or {}
                # Registrar consumo de tokens.
                try:
                    self.token_usage["prompt"] += int(usage.get("prompt_tokens", 0))
                    self.token_usage["completion"] += int(usage.get("completion_tokens", 0))
                    self.token_usage["total"] += int(usage.get("total_tokens", 0))
                    self.token_usage["calls"] += 1
                    model = self.cfg["groq_model"]
                    self.token_usage["by_model"].setdefault(model, {"prompt": 0, "completion": 0, "total": 0, "calls": 0})
                    self.token_usage["by_model"][model]["prompt"] += int(usage.get("prompt_tokens", 0))
                    self.token_usage["by_model"][model]["completion"] += int(usage.get("completion_tokens", 0))
                    self.token_usage["by_model"][model]["total"] += int(usage.get("total_tokens", 0))
                    self.token_usage["by_model"][model]["calls"] += 1
                except (ValueError, TypeError):
                    pass
                dlog(self, "LLM", f"groq completion ok model={self.cfg['groq_model']}",
                     prompt_tokens=usage.get("prompt_tokens", 0),
                     completion_tokens=usage.get("completion_tokens", 0),
                     total_tokens=usage.get("total_tokens", 0))
                message = (data.get("choices") or [{}])[0].get("message", {})
                content = (message.get("content") or "").strip() or None
                calls = message.get("tool_calls") or None
                if content is None and not calls:
                    return (None, None, "empty_response")
                return (content, calls, None)
            except urllib.error.HTTPError as exc:
                if exc.code in (401, 403, 429) or 500 <= exc.code < 600:
                    last_err = f"http_{exc.code}"
                    if index + 1 < len(keys):
                        continue
                return (None, None, f"http_{exc.code}")
            except Exception:
                last_err = "unreachable"
                if index + 1 < len(keys):
                    continue
                return (None, None, "unreachable")
        return (None, None, last_err)

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

    def _complete(self, request_id: str, job: dict, text: str, error: str | None) -> None:
        """Cierra la operación (vía común de stub, LLM y tools)."""
        dlog(self, "COMPLETE", f"request={request_id} error={error} text_len={len(text or '')}")
        with self.lock:
            session = self.sessions.get(job["session_id"])
            if session is None:
                return
            if request_id in self.cancelled:
                self.cancelled.discard(request_id)
                return  # cancelada a mitad: sin efectos
            if time.monotonic() > job["deadline"]:
                error, text = "timeout", ""
            session["updated_at"] = utc_now()
            if error is None:
                session["state"] = "completed"
                session["messages"].append({
                    "requestId": new_id("req"), "sessionId": session["id"],
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
        if tool["requiresConfirmation"]:
            # El tiempo del usuario no consume el plazo de la operación.
            job["deadline"] = time.monotonic() + CONFIRM_TIMEOUT_S + 60
            decision = self.ask_confirmation(job["request_id"], job["session_id"],
                                             name, args, chat_id=job.get("chat_id"))
            if decision != "allow_once":
                return ("denegada por el usuario"
                        if decision == "deny" else "caducada sin respuesta")
        return self.execute_tool(name, args)

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
                           None)
            return
        messages: list[dict] = [
            {"role": "system", "content": SYSTEM_FIXED},
            {"role": "system", "content": self._context_block(channel)},
        ]
        messages.extend(history[-MAX_HISTORY:])
        for turn in range(MAX_TURNS):
            dlog(self, "PROCESS", f"turn {turn+1}/{MAX_TURNS} request={request_id}")
            content, calls, error = self.groq_completion(messages)
            if error is not None:
                dlog(self, "PROCESS", f"groq error={error} request={request_id}")
                self._complete(request_id, job, "", error)
                return
            if not calls:
                dlog(self, "PROCESS", f"final response len={len(content or '')} request={request_id}")
                self._complete(request_id, job, content or "(vacío)", None)
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
        self._complete(request_id, job, "", "max_turns")

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
            self._telegram_call("getMe", {}, 20)
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


def make_handler(brain: Brain, network: ipaddress.IPv4Network, status: dict):
    cors_origin = brain.cfg["cors_origin"]

    class Handler(BaseHTTPRequestHandler):
        server_version = "AtlasRelay/3"

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
            if cors_origin:
                self.send_header("Access-Control-Allow-Origin", cors_origin)
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
            if cors_origin:
                self.send_header("Access-Control-Allow-Origin", cors_origin)
                self.send_header("Access-Control-Allow-Methods", "GET, POST")
                self.send_header("Access-Control-Allow-Headers", "Content-Type")
            self.end_headers()

        def do_GET(self) -> None:  # noqa: N802 (firma de BaseHTTPRequestHandler)
            if not self._lan_only():
                self._json(403, {"ok": False, "error": "same_network_only"})
                return
            parsed = urllib.parse.urlparse(self.path)
            if parsed.path == "/v1/health":
                with brain.lock:
                    queued = len(brain.events)
                self._json(200, {"ok": True, "telegram": status["telegram"],
                                 "llm": status["llm"], "calendar": status["calendar"],
                                 "events": queued})
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
            self._json(404, {"ok": False, "error": "not_found"})

    return Handler


def main() -> int:
    cfg = read_env()
    os.makedirs(cfg["data_dir"], exist_ok=True)

    host = cfg["host"] or lan_ip()
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

    threading.Thread(target=brain.worker, daemon=True, name="brain-worker").start()
    if cfg["token"]:
        threading.Thread(target=brain.telegram_loop, daemon=True,
                         name="telegram-poll").start()

    server = ThreadingHTTPServer((host, cfg["port"]),
                                 make_handler(brain, network, status))
    log(f"escuchando en {host}:{cfg['port']} (solo red local {network}).")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
