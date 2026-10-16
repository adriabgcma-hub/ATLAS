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
  RELAY_HOST                 IP de escucha. Vacío = IP LAN autodetectada.
  RELAY_PORT                 Puerto (defecto 8765).
  RELAY_DATA_DIR             Estado en disco (defecto temp/atlas-relay, dentro
                             del directorio de trabajo).
  RELAY_CORS_ORIGIN          Origen CORS (defecto vacío = sin CORS). Solo dev.

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
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import NoReturn

MAX_CONTENT_LEN = 4000
MAX_BODY_BYTES = 64 * 1024
TELEGRAM_POLL_TIMEOUT_S = 25
TELEGRAM_RETRY_S = 5
GROQ_TIMEOUT_S = 25
OP_TIMEOUT_S = 30
MAX_HISTORY = 6
STORE_HISTORY = 100
DEFAULT_GROQ_MODEL = "openai/gpt-oss-120b"
GROQ_API = "https://api.groq.com/openai/v1/chat/completions"
TELEGRAM_API = "https://api.telegram.org"
# Groq (vía Cloudflare) rechaza el User-Agent por defecto de urllib con 403.
USER_AGENT = "AtlasRelay/3"

TERMINAL_STATES = {"idle", "cancelled", "completed", "failed"}


def log(message: str) -> None:
    print(f"[relay {time.strftime('%H:%M:%S')}] {message}", file=sys.stderr, flush=True)


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

    return {
        "token": os.environ.get("TELEGRAM_BOT_TOKEN", "").strip(),
        "allowed": allowed,
        "groq_keys": groq_keys,
        "groq_model": model,
        "groq_temperature": temperature,
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
        atomic_write(sessions_path, {"sessions": self.sessions, "active_id": self.active_id})

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

    # -- recepción -------------------------------------------------------
    def receive(self, content: str, source: str, chat_id: int | None = None) -> dict:
        """Registra una petición y la encola al worker. Devuelve ids."""
        text = content.strip()
        if not text:
            raise ValueError("empty_message")
        text = text[:MAX_CONTENT_LEN]
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
        with self.lock:
            # El conjunto se poda: solo importan las cancelaciones recientes
            # (mejor esfuerzo; una cancelación tardía se ignora igual).
            if len(self.cancelled) > 2000:
                self.cancelled.clear()
            self.cancelled.add(request_id)
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
                    return True
            return False

    # -- Groq ------------------------------------------------------------
    def groq_chat(self, history: list[dict]) -> tuple[str, str | None]:
        """(texto, error). Sin claves → stub local explícito."""
        keys = self.cfg["groq_keys"]
        if not keys:
            last_user = next(
                (m["content"] for m in reversed(history) if m["role"] == "user"), "")
            return (f"(stub local · sin claves Groq) Recibido: {last_user}", None)
        body = json.dumps({
            "model": self.cfg["groq_model"],
            "temperature": self.cfg["groq_temperature"],
            "messages": [{"role": m["role"], "content": m["content"]}
                         for m in history[-MAX_HISTORY:]],
        }).encode("utf-8")
        last_err: str | None = "unknown"
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
                text = ((data.get("choices") or [{}])[0].get("message", {})
                        .get("content", "") or "").strip()
                if not text:
                    return ("", "empty_response")
                return (text, None)
            except urllib.error.HTTPError as exc:
                if exc.code in (401, 403, 429) or 500 <= exc.code < 600:
                    last_err = f"http_{exc.code}"
                    if index + 1 < len(keys):
                        continue
                return ("", f"http_{exc.code}")
            except Exception:
                last_err = "unreachable"
                if index + 1 < len(keys):
                    continue
                return ("", "unreachable")
        return ("", last_err or "unknown")

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

    def _process(self, job: dict) -> None:
        request_id = job["request_id"]
        with self.lock:
            session = self.sessions.get(job["session_id"])
            if session is None:
                return
            history = [{"role": m["role"], "content": m["content"]}
                       for m in session["messages"]]
        text, error = self.groq_chat(history)
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
                                 "llm": status["llm"], "events": queued})
                return
            if parsed.path == "/v1/session":
                self._json(200, {"session": brain.get_snapshot(),
                                 "cursor": brain.current_seq()})
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
