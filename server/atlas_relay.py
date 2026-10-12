#!/usr/bin/env python3
"""Relay de Atlas (Fase 2): puente entre Telegram y el núcleo.

- Consulta el bot por **polling saliente** (`getUpdates`): nunca abre webhooks,
  el servidor no se expone a internet.
- Sirve la cola FIFO por HTTP **solo en la red local** (loopback o mismo /24).
- Solo stdlib, sin dependencias. Requiere Python >= 3.11.
- Ejecutar desde la raíz del repo (usa `temp/` para el offset, ignorado por Git).

Variables de entorno (valores solo en `.env` local, nunca en Git):
  TELEGRAM_BOT_TOKEN         Token de @BotFather. Vacío = Telegram desactivado.
  TELEGRAM_ALLOWED_USER_IDS  IDs separados por comas. Solo esos hablan al bot.
  RELAY_HOST                 IP de escucha. Vacío = IP LAN autodetectada.
  RELAY_PORT                 Puerto (defecto 8765).
  RELAY_DATA_DIR             Directorio del offset (defecto temp/atlas-relay,
                             debe quedar dentro del directorio de trabajo).
  RELAY_CORS_ORIGIN          Origen permitido para CORS (defecto vacío = sin
                             CORS). Solo para desarrollo web local.

Códigos de salida: 0 ok, 1 fallo en ejecución, 2 error de configuración.
"""

from __future__ import annotations

import ipaddress
import json
import os
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
API = "https://api.telegram.org"


def log(message: str) -> None:
    print(f"[relay] {message}", file=sys.stderr, flush=True)


def fail_config(message: str) -> NoReturn:
    log(f"config: {message}")
    raise SystemExit(2)


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

    return {
        "token": os.environ.get("TELEGRAM_BOT_TOKEN", "").strip(),
        "allowed": allowed,
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


class Inbox:
    """Cola FIFO en memoria. El cursor lo lleva el cliente (`since`)."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._items: deque[dict] = deque()
        self._seq = 0

    def push(self, content: str, source: str) -> dict:
        with self._lock:
            self._seq += 1
            item = {"seq": self._seq, "content": content, "source": source,
                    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
            self._items.append(item)
            if len(self._items) > 500:
                self._items.popleft()
            return item

    def since(self, cursor: int) -> list[dict]:
        with self._lock:
            return [m for m in self._items if m["seq"] > cursor]

    def pending(self) -> int:
        with self._lock:
            return len(self._items)


def telegram_call(token: str, method: str, payload: dict, timeout: int) -> dict:
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        f"{API}/bot{token}/{method}", data=data,
        headers={"Content-Type": "application/json"}, method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as response:
        return json.load(response)


def offset_path(data_dir: str) -> str:
    return os.path.join(data_dir, "telegram-offset.json")


def load_offset(data_dir: str) -> int:
    try:
        with open(offset_path(data_dir), encoding="utf-8") as fh:
            value = json.load(fh).get("offset", 0)
        return int(value) if isinstance(value, int) and value >= 0 else 0
    except (OSError, ValueError, AttributeError):
        return 0


def save_offset(data_dir: str, offset: int) -> None:
    tmp = offset_path(data_dir) + ".tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump({"offset": offset}, fh)
    os.replace(tmp, offset_path(data_dir))


def telegram_loop(token: str, allowed: set[int], inbox: Inbox, data_dir: str) -> None:
    """Bucle de polling. Solo acepta texto de IDs autorizados."""
    try:
        me = telegram_call(token, "getMe", {}, 20)
    except urllib.error.HTTPError as exc:
        if exc.code in (401, 404):
            log("token de Telegram inválido (getMe 401/404). Relay detenido.")
            raise SystemExit(2)
        log(f"getMe falló ({exc.code}), reintentando.")
    except OSError as exc:
        log(f"sin red para getMe ({exc}), reintentando.")

    offset = load_offset(data_dir)
    log(f"Telegram activo (offset {offset}, {len(allowed)} id(s) autorizados).")
    while True:
        try:
            data = telegram_call(
                token, "getUpdates",
                {"offset": offset, "timeout": TELEGRAM_POLL_TIMEOUT_S,
                 "allowed_updates": ["message"]},
                TELEGRAM_POLL_TIMEOUT_S + 10,
            )
        except Exception as exc:  # red o Telegram caídos: backoff y seguir
            log(f"poll falló ({type(exc).__name__}), reintento en {TELEGRAM_RETRY_S}s.")
            time.sleep(TELEGRAM_RETRY_S)
            continue
        for update in data.get("result", []):
            offset = max(offset, int(update.get("update_id", 0)) + 1)
            message = update.get("message", {})
            sender = message.get("from", {}).get("id")
            text = (message.get("text") or message.get("caption") or "").strip()
            if not text or sender not in allowed:
                continue
            if len(text) > MAX_CONTENT_LEN:
                text = text[:MAX_CONTENT_LEN]
            inbox.push(text, "telegram")
        save_offset(data_dir, offset)


def make_handler(inbox: Inbox, network: ipaddress.IPv4Network, telegram_mode: str,
                 cors_origin: str):
    class Handler(BaseHTTPRequestHandler):
        server_version = "AtlasRelay/2"

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

        def _json(self, status: int, payload: dict) -> None:
            body = json.dumps(payload).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            if cors_origin:
                self.send_header("Access-Control-Allow-Origin", cors_origin)
            self.end_headers()
            self.wfile.write(body)

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
                self._json(200, {"ok": True, "telegram": telegram_mode,
                                 "pending": inbox.pending()})
                return
            if parsed.path == "/v1/messages":
                query = urllib.parse.parse_qs(parsed.query)
                try:
                    since = int(query.get("since", ["0"])[0])
                    if since < 0:
                        raise ValueError
                except ValueError:
                    self._json(400, {"ok": False, "error": "bad_since"})
                    return
                self._json(200, {"messages": inbox.since(since)})
                return
            self._json(404, {"ok": False, "error": "not_found"})

        def do_POST(self) -> None:  # noqa: N802 (firma de BaseHTTPRequestHandler)
            if not self._lan_only():
                self._json(403, {"ok": False, "error": "same_network_only"})
                return
            if urllib.parse.urlparse(self.path).path != "/v1/inject":
                self._json(404, {"ok": False, "error": "not_found"})
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                length = 0
            if length <= 0 or length > MAX_BODY_BYTES:
                self._json(400, {"ok": False, "error": "bad_body"})
                return
            try:
                payload = json.loads(self.rfile.read(length))
            except ValueError:
                self._json(400, {"ok": False, "error": "bad_json"})
                return
            content = payload.get("content", "")
            source = payload.get("source", "telegram")
            if not isinstance(content, str) or not content.strip():
                self._json(400, {"ok": False, "error": "empty_message"})
                return
            if source not in ("local", "telegram"):
                self._json(400, {"ok": False, "error": "bad_source"})
                return
            item = inbox.push(content.strip()[:MAX_CONTENT_LEN], source)
            self._json(201, {"ok": True, "seq": item["seq"]})

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

    inbox = Inbox()
    telegram_mode = "disabled"
    if cfg["token"]:
        if not cfg["allowed"]:
            fail_config("Con bot activo, TELEGRAM_ALLOWED_USER_IDS no puede estar vacío.")
        telegram_mode = "enabled"
        thread = threading.Thread(
            target=telegram_loop,
            args=(cfg["token"], cfg["allowed"], inbox, cfg["data_dir"]),
            daemon=True, name="telegram-poll",
        )
        thread.start()
    else:
        log("sin TELEGRAM_BOT_TOKEN: Telegram desactivado (solo /v1/inject).")

    server = ThreadingHTTPServer((host, cfg["port"]),
                                         make_handler(inbox, network, telegram_mode,
                                                      cfg["cors_origin"]))
    log(f"escuchando en {host}:{cfg['port']} (solo red local {network}).")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
