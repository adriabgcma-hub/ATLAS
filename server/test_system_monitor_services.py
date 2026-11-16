"""Estado de servicios (PANEL_SERVICE_PROBES) expuesto por el snapshot del panel."""

import os
import socket
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest import mock

import system_monitor

# URLs de ejemplo tal y como se configuran en el .env del relay: nombre del
# host (se abren desde el Mac), nunca loopback. El código no lo exige.
NEXTCLOUD = "http://atlas-hub:8080"
JELLYFIN = "http://atlas-hub:8096"


class _HandlerVivo(BaseHTTPRequestHandler):
    """Responde 200 a cualquier GET (servicio "arriba")."""

    def do_GET(self):  # noqa: N802 (nombre impuesto por la stdlib)
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")

    def log_message(self, *args):  # silencia el log a stderr
        pass


def _servidor_vivo() -> tuple[ThreadingHTTPServer, threading.Thread, str]:
    """Servidor HTTP local efímero en un hilo; (servidor, hilo, url)."""
    server = ThreadingHTTPServer(("127.0.0.1", 0), _HandlerVivo)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, thread, f"http://127.0.0.1:{server.server_address[1]}"


def _url_puerto_cerrado() -> str:
    """URL de loopback en un puerto que ya no escucha a nadie."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    return f"http://127.0.0.1:{port}"


class TestServiceProbes(unittest.TestCase):
    def test_parsea_probes(self):
        raw = f"nextcloud={NEXTCLOUD}, jellyfin={JELLYFIN}"
        with mock.patch.dict(os.environ,
                             {"PANEL_SERVICE_PROBES": raw}, clear=True):
            mon = system_monitor.SystemMonitor()
        self.assertEqual(mon.service_probes, {
            "nextcloud": NEXTCLOUD,
            "jellyfin": JELLYFIN,
        })

    def test_descarta_trozos_invalidos(self):
        raw = ("sin_igual,=sin_nombre,sin_url=ftp://x,"
               "nextcloud=" + NEXTCLOUD)
        with mock.patch.dict(os.environ,
                             {"PANEL_SERVICE_PROBES": raw}, clear=True):
            mon = system_monitor.SystemMonitor()
        self.assertEqual(mon.service_probes, {"nextcloud": NEXTCLOUD})

    def test_sin_probes_vacio(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            mon = system_monitor.SystemMonitor()
        self.assertEqual(mon.service_probes, {})

    def test_servicios_arriba_y_abajo(self):
        server, thread, url_vivo = _servidor_vivo()
        try:
            url_caido = _url_puerto_cerrado()
            raw = f"vivo={url_vivo},caido={url_caido}"
            with mock.patch.dict(os.environ,
                                 {"PANEL_SERVICE_PROBES": raw}, clear=True):
                mon = system_monitor.SystemMonitor()
            servicios = {s["name"]: s for s in mon._services()}
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
        self.assertEqual(sorted(servicios), ["caido", "vivo"])
        self.assertTrue(servicios["vivo"]["up"])
        self.assertEqual(servicios["vivo"]["url"], url_vivo)
        self.assertFalse(servicios["caido"]["up"])
        self.assertEqual(servicios["caido"]["url"], url_caido)


if __name__ == "__main__":
    unittest.main()
