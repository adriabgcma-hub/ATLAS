"""Isolated, authenticated Hefesto v2 journal (echo and read-only tools) and HTTPS listener."""

from __future__ import annotations

import hashlib
import json
import re
import socket
import sqlite3
import ssl
import threading
import time
import uuid
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, HTTPServer
from socketserver import ThreadingMixIn
from types import MappingProxyType
MAX_BODY_BYTES = 128 * 1024
LEASE_SECONDS = 15
INVOCATION_SECONDS = 15
CONNECTION_TIMEOUT = 3.0
REQUEST_DEADLINE = 5.0
_ID = re.compile(r"[A-Za-z0-9_-]{1,80}\Z", re.ASCII)
_FINGERPRINT = re.compile(r"[0-9a-f]{64}\Z", re.ASCII)
_PREFIX = "/v1/hefesto/"
_FIELDS = {
    "register": {"instanceId"},
    "heartbeat": {"instanceId"},
    "control": {"action", "expectedGeneration"},
    "outcome": {"invocationId"},
    "submit": {"submissionId", "requestId", "sessionId", "taskId", "toolId", "arguments"},
    "next": {"instanceId", "generation"},
    "result": {"instanceId", "generation", "invocationId", "output"},
}


def _is_id(value):
    return type(value) is str and _ID.fullmatch(value) is not None


@dataclass(frozen=True)
class Peer:
    role: str
    device_id: str

    def __post_init__(self):
        if type(self.role) is not str or self.role not in ("device", "controller") or not _is_id(self.device_id):
            raise ValueError("invalid_peer")


class _Rejected(Exception):
    def __init__(self, code, status=400):
        self.code = code
        self.status = status


def _error(code):
    return {"protocolVersion": 2, "error": code}


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate_key")
        result[key] = value
    return result


def _constant(_value):
    raise ValueError("invalid_json")


def _decode(raw):
    try:
        return json.loads(raw.decode("utf-8"), object_pairs_hook=_pairs, parse_constant=_constant)
    except (ValueError, UnicodeError, RecursionError):
        raise _Rejected("invalid_json") from None


def load_peers(path):
    """Load a fingerprint -> Peer registry; never disclose configuration contents."""
    try:
        with open(path, "rb") as source:
            value = _decode(source.read())
        if type(value) is not dict:
            raise ValueError
        peers = {}
        for fingerprint, record in value.items():
            if not _FINGERPRINT.fullmatch(fingerprint):
                raise ValueError
            if type(record) is not dict or set(record) != {"role", "deviceId"}:
                raise ValueError
            peers[fingerprint] = Peer(record["role"], record["deviceId"])
        return peers
    except Exception:
        raise ValueError("invalid_peer_registry") from None


def _validate(method, path, raw):
    if type(raw) is not bytes:
        raise _Rejected("invalid_body")
    if len(raw) > MAX_BODY_BYTES:
        raise _Rejected("body_too_large", 413)
    if type(path) is not str or not path.startswith(_PREFIX):
        raise _Rejected("not_found", 404)
    operation = path[len(_PREFIX):]
    if operation not in _FIELDS and operation != "status":
        raise _Rejected("not_found", 404)
    if method != ("GET" if operation == "status" else "POST"):
        raise _Rejected("method_not_allowed", 405)
    if operation == "status":
        if raw:
            raise _Rejected("invalid_body")
        return operation, {}
    body = _decode(raw)
    if type(body) is not dict or set(body) != _FIELDS[operation] | {"protocolVersion"}:
        raise _Rejected("invalid_fields")
    if type(body["protocolVersion"]) is not int or body["protocolVersion"] != 2:
        raise _Rejected("invalid_version")
    for key in ("instanceId", "submissionId", "requestId", "sessionId", "taskId", "invocationId"):
        if key in body and not _is_id(body[key]):
            raise _Rejected("invalid_id")
    for key in ("generation", "expectedGeneration"):
        if key in body and (type(body[key]) is not int or not 0 <= body[key] <= 2**63 - 1):
            raise _Rejected("invalid_generation")
    if operation == "control" and (type(body["action"]) is not str or body["action"] not in ("resume", "abort")):
        raise _Rejected("invalid_action")
    if operation == "submit" and body["toolId"] not in ("hefesto.echo", "hefesto.list_dir", "hefesto.read_file"):
        raise _Rejected("unsupported_tool")
    if operation == "submit":
        args = body["arguments"]
        if body["toolId"] == "hefesto.echo":
            if type(args) is not dict or set(args) != {"text"} or type(args["text"]) is not str:
                raise _Rejected("invalid_text")
            try:
                if len(args["text"].encode("utf-8")) > 256:
                    raise _Rejected("invalid_text")
            except UnicodeError:
                raise _Rejected("invalid_text") from None
        else:
            if type(args) is not dict or set(args) != {"path"} or type(args["path"]) is not str:
                raise _Rejected("invalid_path")
            path = args["path"]
            try:
                if not path.startswith("/") or "\x00" in path or len(path.encode("utf-8")) > 1024:
                    raise _Rejected("invalid_path")
            except UnicodeError:
                raise _Rejected("invalid_path") from None
    if "output" in body:
        value = body["output"]
        if type(value) is not dict or set(value) != {"text"} or type(value["text"]) is not str:
            raise _Rejected("invalid_text")
        try:
            if len(value["text"].encode("utf-8")) > 64 * 1024:
                raise _Rejected("invalid_text")
        except UnicodeError:
            raise _Rejected("invalid_text") from None
    return operation, body


_SCHEMA = """
CREATE TABLE IF NOT EXISTS devices (
    device_id TEXT PRIMARY KEY,
    instance_id TEXT,
    generation INTEGER NOT NULL DEFAULT 0,
    suspended INTEGER NOT NULL DEFAULT 1,
    lease_mono REAL
);
CREATE TABLE IF NOT EXISTS used_instances (
    device_id TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    PRIMARY KEY (device_id, instance_id)
);
CREATE TABLE IF NOT EXISTS invocations (
    invocation_id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    submission TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    request_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    text TEXT NOT NULL,
    expires_ms INTEGER NOT NULL,
    deadline_mono REAL NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending','dispatched','succeeded','cancelled','expired','unknown')),
    output_text TEXT,
    UNIQUE (device_id, submission_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS one_active_invocation
    ON invocations(device_id) WHERE state IN ('pending', 'dispatched');
"""


class Journal:
    """One caller-owned journal per database; all operations are serialized."""

    def __init__(self, path, *, monotonic=time.monotonic, wall_time=time.time):
        self._lock = threading.RLock()
        self._monotonic = monotonic
        self._wall_time = wall_time
        self._db = None
        try:
            self._db = sqlite3.connect(path, check_same_thread=False, isolation_level=None)
            self._db.row_factory = sqlite3.Row
            self._db.execute("PRAGMA journal_mode=WAL")
            self._db.execute("PRAGMA synchronous=FULL")
            self._db.executescript(_SCHEMA)
            self._db.execute("BEGIN IMMEDIATE")
            self._db.execute("UPDATE invocations SET state='cancelled' WHERE state='pending'")
            self._db.execute("UPDATE invocations SET state='unknown' WHERE state='dispatched'")
            self._db.execute("""UPDATE devices SET generation=generation + (instance_id IS NOT NULL),
                             instance_id=NULL, lease_mono=NULL, suspended=1""")
            self._db.execute("COMMIT")
        except Exception:
            self.close()
            raise ValueError("journal_unavailable") from None

    def close(self):
        with self._lock:
            if self._db is not None:
                self._db.close()
                self._db = None

    def dispatch(self, peer: Peer, method: str, path: str, body: bytes = b"") -> tuple[int, dict]:
        """Return (HTTP status, JSON object). The caller authenticates Peer via TLS."""
        if type(peer) is not Peer:
            return 403, _error("unauthorized_peer")
        with self._lock:
            if self._db is None:
                return 503, _error("journal_unavailable")
            try:
                self._db.execute("BEGIN IMMEDIATE")
                now, wall = self._monotonic(), self._wall_time()
                self._sweep(now, wall)
                try:
                    operation, data = _validate(method, path, body)
                    if ((operation in ("register", "heartbeat", "next", "result") and peer.role != "device")
                            or (operation in ("submit", "outcome") and peer.role != "controller")):
                        raise _Rejected("forbidden", 403)
                    self._db.execute("INSERT OR IGNORE INTO devices(device_id) VALUES (?)", (peer.device_id,))
                    response = self._operate(peer, operation, data, now, wall)
                    result = (200, response)
                except _Rejected as exc:
                    result = (exc.status, _error(exc.code))
                self._db.execute("COMMIT")
                return result
            except Exception:
                if self._db.in_transaction:
                    self._db.rollback()
                return 503, _error("journal_unavailable")

    def _cancel_active(self, device_id):
        self._db.execute("""UPDATE invocations SET state=CASE state
                         WHEN 'pending' THEN 'cancelled' ELSE 'unknown' END
                         WHERE device_id=? AND state IN ('pending','dispatched')""", (device_id,))

    def _sweep(self, now, wall):
        for device in self._db.execute(
                "SELECT device_id FROM devices WHERE instance_id IS NOT NULL AND lease_mono<=?", (now,)).fetchall():
            self._cancel_active(device["device_id"])
            self._db.execute("""UPDATE devices SET instance_id=NULL, lease_mono=NULL,
                             generation=generation+1, suspended=1 WHERE device_id=?""", (device["device_id"],))
        self._db.execute("""UPDATE invocations SET state=CASE state
                         WHEN 'pending' THEN 'expired' ELSE 'unknown' END
                         WHERE state IN ('pending','dispatched') AND (deadline_mono<=? OR expires_ms<=?)""",
                         (now, wall * 1000))

    def _device(self, device_id):
        return self._db.execute("SELECT * FROM devices WHERE device_id=?", (device_id,)).fetchone()

    def _active(self, device_id):
        return self._db.execute("SELECT * FROM invocations WHERE device_id=? AND state IN ('pending','dispatched')",
                                (device_id,)).fetchone()

    def _status(self, device_id):
        device, active = self._device(device_id), self._active(device_id)
        return {"protocolVersion": 2, "deviceId": device_id, "instanceId": device["instance_id"],
                "generation": device["generation"], "suspended": bool(device["suspended"]),
                "online": device["instance_id"] is not None,
                "activeInvocationId": active["invocation_id"] if active else None}

    @staticmethod
    def _ack(invocation):
        return {"protocolVersion": 2, "invocationId": invocation["invocation_id"], "state": invocation["state"]}

    @staticmethod
    def _live(device, instance=None, generation=None, *, ready=False):
        if device["instance_id"] is None:
            raise _Rejected("device_offline", 409)
        if instance is not None and instance != device["instance_id"]:
            raise _Rejected("instance_mismatch", 409)
        if generation is not None and generation != device["generation"]:
            raise _Rejected("stale_generation", 409)
        if ready and device["suspended"]:
            raise _Rejected("device_suspended", 409)

    def _operate(self, peer, operation, data, now, wall):
        device_id = peer.device_id
        device = self._device(device_id)
        if operation == "status":
            return self._status(device_id)
        if operation == "register":
            instance = data["instanceId"]
            if device["instance_id"] == instance:
                # Only heartbeat renews a live registration's lease.
                return self._status(device_id)
            if device["instance_id"] is not None:
                raise _Rejected("instance_conflict", 409)
            if self._db.execute("SELECT 1 FROM used_instances WHERE device_id=? AND instance_id=?",
                                (device_id, instance)).fetchone():
                raise _Rejected("instance_invalidated", 409)
            self._db.execute("INSERT INTO used_instances VALUES (?,?)", (device_id, instance))
            self._db.execute("""UPDATE devices SET instance_id=?, generation=generation+1,
                             suspended=1, lease_mono=? WHERE device_id=?""", (instance, now + LEASE_SECONDS, device_id))
            return self._status(device_id)
        if operation == "heartbeat":
            self._live(device, data["instanceId"])
            self._db.execute("UPDATE devices SET lease_mono=? WHERE device_id=?", (now + LEASE_SECONDS, device_id))
            return self._status(device_id)
        if operation == "control":
            if data["expectedGeneration"] != device["generation"]:
                raise _Rejected("stale_generation", 409)
            if data["action"] == "resume":
                self._live(device)
                if not device["suspended"]:
                    return self._status(device_id)
            self._cancel_active(device_id)
            self._db.execute("UPDATE devices SET suspended=?, generation=generation+1 WHERE device_id=?",
                             (int(data["action"] == "abort"), device_id))
            return self._status(device_id)
        if operation == "submit":
            submission = json.dumps(data, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
            previous = self._db.execute("SELECT * FROM invocations WHERE device_id=? AND submission_id=?",
                                        (device_id, data["submissionId"])).fetchone()
            if previous is not None:
                if submission != previous["submission"]:
                    raise _Rejected("submission_conflict", 409)
                return self._ack(previous)
            self._live(device, ready=True)
            if self._active(device_id) is not None:
                raise _Rejected("device_busy", 409)
            invocation_id = uuid.uuid4().hex
            self._db.execute("""INSERT INTO invocations
                             (invocation_id, device_id, submission_id, submission, instance_id, generation,
                              request_id, session_id, task_id, text, expires_ms, deadline_mono, state)
                             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,'pending')""",
                             (invocation_id, device_id, data["submissionId"], submission, device["instance_id"],
                              device["generation"], data["requestId"], data["sessionId"], data["taskId"],
                              data["arguments"].get("text", ""), int(wall * 1000) + INVOCATION_SECONDS * 1000,
                              now + INVOCATION_SECONDS))
            return {"protocolVersion": 2, "invocationId": invocation_id, "state": "pending"}
        if operation == "outcome":
            invocation = self._db.execute(
                "SELECT * FROM invocations WHERE invocation_id=? AND device_id=?",
                (data["invocationId"], device_id)).fetchone()
            if invocation is None:
                raise _Rejected("invocation_not_found", 404)
            return {"protocolVersion": 2, "invocationId": invocation["invocation_id"],
                    "state": invocation["state"], "outputText": invocation["output_text"]}
        if operation == "next":
            self._live(device, data["instanceId"], data["generation"], ready=True)
            active = self._active(device_id)
            invocation = None
            if active is not None:
                submitted = json.loads(active["submission"])
                self._db.execute("UPDATE invocations SET state='dispatched' WHERE invocation_id=?",
                                 (active["invocation_id"],))
                invocation = {"protocolVersion": 2, "invocationId": active["invocation_id"],
                              "requestId": active["request_id"], "sessionId": active["session_id"],
                              "taskId": active["task_id"], "deviceId": device_id,
                              "instanceId": active["instance_id"], "generation": active["generation"],
                              "toolId": submitted["toolId"], "arguments": submitted["arguments"],
                              "expiresAtUnixMs": active["expires_ms"]}
            return {"protocolVersion": 2, "invocation": invocation}
        invocation = self._db.execute("SELECT * FROM invocations WHERE invocation_id=? AND device_id=?",
                                      (data["invocationId"], device_id)).fetchone()
        if invocation is None:
            raise _Rejected("invocation_not_found", 404)
        if data["instanceId"] != invocation["instance_id"]:
            raise _Rejected("instance_mismatch", 409)
        if data["generation"] != invocation["generation"]:
            raise _Rejected("stale_generation", 409)
        submitted = json.loads(invocation["submission"])
        if submitted["toolId"] == "hefesto.echo" and data["output"]["text"] != invocation["text"]:
            raise _Rejected("result_conflict", 409)
        # A committed success survives ACK loss, expiry, abort and relay restart.
        if invocation["state"] == "succeeded":
            if data["output"]["text"] != invocation["output_text"]:
                raise _Rejected("result_conflict", 409)
            return self._ack(invocation)
        if invocation["state"] != "dispatched":
            raise _Rejected("invocation_not_dispatched", 409)
        self._live(device, data["instanceId"], data["generation"], ready=True)
        self._db.execute("UPDATE invocations SET state='succeeded', output_text=? WHERE invocation_id=?",
                         (data["output"]["text"], invocation["invocation_id"]))
        return {"protocolVersion": 2, "invocationId": invocation["invocation_id"], "state": "succeeded"}


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, _format, *args):
        pass

    def send_error(self, code, message=None, explain=None):
        self._send(code, _error("invalid_http_request"))

    def handle_expect_100(self):
        self._send(400, _error("invalid_http_framing"))
        return False

    def _send(self, status, value):
        self.close_connection = True
        raw = json.dumps(value, separators=(",", ":"), ensure_ascii=True).encode("utf-8")
        self.send_response_only(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Connection", "close")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(raw)

    def parse_request(self):
        if not super().parse_request():
            return False
        names = [name.lower() for name in self.headers.keys()]
        if (self.request_version not in ("HTTP/1.0", "HTTP/1.1")
                or len(names) != len(set(names))
                or any(name in self.headers for name in ("Transfer-Encoding", "Trailer", "Expect", "Upgrade"))
                or any("\r" in value or "\n" in value for value in self.headers.values())
                or self.headers.defects
                or (self.request_version == "HTTP/1.1" and not self.headers.get("Host"))):
            self._send(400, _error("invalid_http_framing"))
            return False
        # BaseHTTPRequestHandler normalizes // paths; require an exact origin-form target.
        if self.requestline.split()[1] != self.path:
            self._send(400, _error("invalid_http_framing"))
            return False
        return True

    def _handle(self):
        try:
            peer = getattr(self.connection, "peer", None)
            if peer is None:
                raise _Rejected("unauthorized_peer", 403)
            length = self.headers.get("Content-Length")
            if length is not None and (not re.fullmatch(r"[0-9]+", length, re.ASCII)):
                raise _Rejected("invalid_http_framing")
            if length is not None and (len(length) > 6 or int(length) > MAX_BODY_BYTES):
                raise _Rejected("body_too_large", 413)
            if self.command == "GET":
                if length is not None and int(length) != 0:
                    raise _Rejected("invalid_body")
                raw = b""
            else:
                if length is None:
                    raise _Rejected("length_required", 411)
                if (self.headers.get_content_type() != "application/json"
                        or self.headers.get_content_charset("utf-8") != "utf-8"):
                    raise _Rejected("invalid_content_type", 415)
                raw = self.rfile.read(int(length))
                if len(raw) != int(length):
                    raise _Rejected("invalid_body")
            status, value = self.server.journal.dispatch(peer, self.command, self.path, raw)
        except _Rejected as exc:
            status, value = exc.status, _error(exc.code)
        except TimeoutError:
            status, value = 408, _error("request_timeout")
        self._send(status, value)

    do_GET = _handle
    do_POST = _handle


class _Server(ThreadingMixIn, HTTPServer):
    daemon_threads = False
    block_on_close = True
    request_queue_size = 32

    def __init__(self, address, journal, peers, context):
        self.journal = journal
        self.peers = MappingProxyType(dict(peers))
        self._context = context
        self._connections = {}
        self._connections_lock = threading.Lock()
        self._closing = False
        self._slots = threading.BoundedSemaphore(32)
        self._watchdog = None
        self._watchdog_stop = threading.Event()
        self.request_deadline = REQUEST_DEADLINE
        super().__init__(address, _Handler)

    def process_request(self, request, client_address):
        if not self._slots.acquire(blocking=False):
            request.close()
            return
        try:
            super().process_request(request, client_address)
        except Exception:
            self._slots.release()
            request.close()

    def process_request_thread(self, request, client_address):
        connection = request
        try:
            with self._connections_lock:
                if self._closing:
                    return
                request.settimeout(CONNECTION_TIMEOUT)
                connection = self._context.wrap_socket(request, server_side=True, do_handshake_on_connect=False)
                self._connections[connection] = time.monotonic() + self.request_deadline
                self._start_watchdog_locked()
            connection.do_handshake()
            certificate = connection.getpeercert(binary_form=True)
            peer = self.peers.get(hashlib.sha256(certificate).hexdigest()) if certificate else None
            if peer is None:
                # Unlisted identities are dropped before any HTTP byte is read or written.
                return
            connection.peer = peer
            super().process_request_thread(connection, client_address)
        except (OSError, ValueError):
            pass
        finally:
            with self._connections_lock:
                self._connections.pop(connection, None)
            connection.close()
            request.close()
            self._slots.release()

    def _start_watchdog_locked(self):
        if self._watchdog is None:
            self._watchdog = threading.Thread(target=self._watch, name="hefesto-deadline", daemon=True)
            self._watchdog.start()

    def _watch(self):
        # Per-operation socket timeouts cannot bound a trickle inside one TLS record;
        # an absolute deadline closes the socket no matter how bytes are paced.
        while not self._watchdog_stop.wait(0.2):
            now = time.monotonic()
            with self._connections_lock:
                expired = [connection for connection, deadline in self._connections.items() if deadline <= now]
            for connection in expired:
                try:
                    connection.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass

    def handle_error(self, request, client_address):
        pass

    def server_close(self):
        with self._connections_lock:
            self._closing = True
            connections = list(self._connections)
            watchdog = self._watchdog
        for connection in connections:
            try:
                connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self._watchdog_stop.set()
        if watchdog is not None:
            watchdog.join(timeout=2)
        super().server_close()


def create_server(address, journal, peers, *, certfile, keyfile, cafile):
    """Bind an mTLS listener; caller owns serving, shutdown, close and journal."""
    try:
        if type(peers) is not dict or any(type(key) is not str or not _FINGERPRINT.fullmatch(key)
                                          or type(value) is not Peer for key, value in peers.items()):
            raise ValueError
        if not certfile or not keyfile or not cafile:
            raise ValueError
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.minimum_version = ssl.TLSVersion.TLSv1_2
        context.verify_mode = ssl.CERT_REQUIRED
        context.load_verify_locations(cafile=cafile)
        context.load_cert_chain(certfile=certfile, keyfile=keyfile, password=lambda: b"")
    except Exception:
        raise ValueError("tls_configuration_error") from None
    try:
        return _Server(address, journal, peers, context)
    except Exception:
        raise ValueError("server_unavailable") from None
