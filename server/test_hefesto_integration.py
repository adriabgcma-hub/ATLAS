import http.client
import ipaddress
import json
import socket
import tempfile
import threading
import unittest
from http.server import ThreadingHTTPServer
from pathlib import Path
from unittest.mock import Mock, patch

import atlas_relay
from test_hefesto import TLSFixture


SETTINGS = {
    "HEFESTO_PORT": "8766",
    "HEFESTO_TLS_CERT": "fixture-cert.pem",
    "HEFESTO_TLS_KEY": "fixture-key.pem",
    "HEFESTO_TLS_CA": "fixture-ca.pem",
    "HEFESTO_PEERS_FILE": "fixture-peers.json",
}


class ListenerLifecycleTests(unittest.TestCase):
    def test_disabled_by_default_opens_nothing(self):
        with patch.dict("os.environ", {}, clear=True), \
                patch.object(atlas_relay.hefesto, "Journal") as journal, \
                patch.object(atlas_relay.hefesto, "create_server") as create:
            with atlas_relay.hefesto_listener("127.0.0.1", "unused"):
                pass
            journal.assert_not_called()
            create.assert_not_called()

    def test_partial_or_invalid_configuration_fails_closed(self):
        cases = [{"HEFESTO_PORT": "8766"}, {"HEFESTO_PORT": ""}]
        cases.extend({**SETTINGS, "HEFESTO_PORT": value}
                     for value in ("0", "65536", "not-a-port"))
        for settings in cases:
            with self.subTest(settings=settings), \
                    patch.dict("os.environ", settings, clear=True), \
                    patch.object(atlas_relay, "log"), \
                    patch.object(atlas_relay.hefesto, "Journal") as journal, \
                    patch.object(atlas_relay.hefesto, "create_server") as create:
                with self.assertRaises(SystemExit) as error:
                    with atlas_relay.hefesto_listener("127.0.0.1", "unused"):
                        self.fail("Invalid settings must not start the listener")
                self.assertEqual(error.exception.code, 2)
                journal.assert_not_called()
                create.assert_not_called()

    def test_failed_tls_setup_closes_journal_without_logging_paths(self):
        with patch.dict("os.environ", SETTINGS, clear=True), \
                patch.object(atlas_relay, "log") as log, \
                patch.object(atlas_relay.hefesto, "load_peers", return_value={}), \
                patch.object(atlas_relay.hefesto, "Journal") as journal, \
                patch.object(atlas_relay.hefesto, "create_server",
                             side_effect=OSError("/private/key.pem")):
            with self.assertRaises(SystemExit):
                with atlas_relay.hefesto_listener("127.0.0.1", "unused"):
                    self.fail("TLS setup failed")
            journal.return_value.close.assert_called_once_with()
            self.assertNotIn("/private", str(log.call_args_list))

    def test_owned_thread_stops_before_server_and_journal_close(self):
        stopped = threading.Event()
        running = threading.Event()
        events = []
        server = Mock()
        server.serve_forever.side_effect = lambda: (running.set(), stopped.wait(5))
        server.shutdown.side_effect = lambda: (events.append("shutdown"), stopped.set())

        def close_server():
            self.assertFalse(any(t.name == "hefesto-https" for t in threading.enumerate()))
            events.append("server_close")

        server.server_close.side_effect = close_server
        with patch.dict("os.environ", SETTINGS, clear=True), \
                patch.object(atlas_relay, "log"), \
                patch.object(atlas_relay.hefesto, "load_peers", return_value={}), \
                patch.object(atlas_relay.hefesto, "Journal") as journal, \
                patch.object(atlas_relay.hefesto, "create_server", return_value=server):
            journal.return_value.close.side_effect = lambda: events.append("journal_close")
            with self.assertRaisesRegex(RuntimeError, "test interrupt"):
                with atlas_relay.hefesto_listener("127.0.0.1", "unused"):
                    self.assertTrue(running.wait(2))
                    self.assertTrue(any(t.name == "hefesto-https" for t in threading.enumerate()))
                    raise RuntimeError("test interrupt")
        self.assertEqual(events, ["shutdown", "server_close", "journal_close"])

    def test_interruption_during_thread_start_still_cleans_up(self):
        stopped = threading.Event()
        running = threading.Event()
        server = Mock()
        server.serve_forever.side_effect = lambda: (running.set(), stopped.wait(5))
        server.shutdown.side_effect = stopped.set
        original_start = threading.Thread.start

        def interrupted_start(thread):
            original_start(thread)
            self.assertTrue(running.wait(2))
            raise KeyboardInterrupt

        with patch.dict("os.environ", SETTINGS, clear=True), \
                patch.object(atlas_relay, "log"), \
                patch.object(atlas_relay.hefesto, "load_peers", return_value={}), \
                patch.object(atlas_relay.hefesto, "Journal") as journal, \
                patch.object(atlas_relay.hefesto, "create_server", return_value=server), \
                patch.object(threading.Thread, "start", interrupted_start):
            with self.assertRaises(KeyboardInterrupt):
                with atlas_relay.hefesto_listener("127.0.0.1", "unused"):
                    self.fail("Thread start was interrupted")
            self.assertFalse(any(t.name == "hefesto-https" for t in threading.enumerate()))
            server.shutdown.assert_called_once_with()
            server.server_close.assert_called_once_with()
            journal.return_value.close.assert_called_once_with()

    def test_real_listener_authenticates_and_closes_its_socket(self):
        fixture = TLSFixture()
        self.addCleanup(fixture.close)
        registry = fixture.root / "peers.json"
        registry.write_text(json.dumps({fixture.fingerprint("device"): {
            "role": "device", "deviceId": "mac-one"}}))
        settings = {**SETTINGS, "HEFESTO_TLS_CERT": fixture.file("server", ".pem"),
                    "HEFESTO_TLS_KEY": fixture.file("server", ".key"),
                    "HEFESTO_TLS_CA": fixture.file("ca", ".pem"),
                    "HEFESTO_PEERS_FILE": str(registry)}
        create = atlas_relay.hefesto.create_server
        servers = []

        def ephemeral_server(address, *args, **kwargs):
            server = create((address[0], 0), *args, **kwargs)
            servers.append(server)
            return server

        with patch.dict("os.environ", settings, clear=True), \
                patch.object(atlas_relay, "log"), \
                patch.object(atlas_relay.hefesto, "create_server", ephemeral_server):
            with atlas_relay.hefesto_listener("127.0.0.1", str(fixture.root)):
                address = servers[0].server_address
                conn = http.client.HTTPSConnection(*address, context=fixture.context("device"), timeout=3)
                try:
                    conn.request("GET", "/v1/hefesto/status")
                    response = conn.getresponse()
                    self.assertEqual(response.status, 200)
                    self.assertEqual(json.loads(response.read())["deviceId"], "mac-one")
                finally:
                    conn.close()
        with socket.socket() as closed:
            self.assertNotEqual(closed.connect_ex(address), 0)
        self.assertFalse(any(t.name == "hefesto-https" for t in threading.enumerate()))
        self.assertTrue(Path(fixture.root, "hefesto.sqlite").is_file())


class ListenerYieldTests(unittest.TestCase):
    def test_deshabilitado_devuelve_none(self):
        with patch.dict("os.environ", {}, clear=True):
            with atlas_relay.hefesto_listener("127.0.0.1", "unused") as ctx:
                self.assertIsNone(ctx)

    def test_habilitado_entrega_journal_y_dispositivo_unico(self):
        peers = {"a" * 64: atlas_relay.hefesto.Peer("device", "mac-one"),
                 "b" * 64: atlas_relay.hefesto.Peer("controller", "mac-one")}
        with patch.dict("os.environ", SETTINGS, clear=True), \
                patch.object(atlas_relay, "log"), \
                patch.object(atlas_relay.hefesto, "load_peers", return_value=peers), \
                patch.object(atlas_relay.hefesto, "Journal") as journal, \
                patch.object(atlas_relay.hefesto, "create_server") as create:
            with atlas_relay.hefesto_listener("127.0.0.1", "unused") as ctx:
                self.assertEqual(ctx, {"journal": journal.return_value,
                                       "device_id": "mac-one"})
            create.assert_called_once()

    def test_sin_dispositivo_entrega_journal_con_device_none(self):
        peers = {"a" * 64: atlas_relay.hefesto.Peer("controller", "mac-one")}
        with patch.dict("os.environ", SETTINGS, clear=True), \
                patch.object(atlas_relay, "log"), \
                patch.object(atlas_relay.hefesto, "load_peers", return_value=peers), \
                patch.object(atlas_relay.hefesto, "Journal") as journal, \
                patch.object(atlas_relay.hefesto, "create_server") as create:
            with atlas_relay.hefesto_listener("127.0.0.1", "unused") as ctx:
                self.assertEqual(ctx, {"journal": journal.return_value,
                                       "device_id": None})
            create.assert_called_once()

    def test_mas_de_un_dispositivo_falla_la_configuracion(self):
        peers = {"a" * 64: atlas_relay.hefesto.Peer("device", "mac-one"),
                 "b" * 64: atlas_relay.hefesto.Peer("device", "mac-two")}
        with patch.dict("os.environ", SETTINGS, clear=True), \
                patch.object(atlas_relay, "log"), \
                patch.object(atlas_relay.hefesto, "load_peers", return_value=peers), \
                patch.object(atlas_relay.hefesto, "Journal") as journal, \
                patch.object(atlas_relay.hefesto, "create_server") as create:
            with self.assertRaises(SystemExit) as error:
                with atlas_relay.hefesto_listener("127.0.0.1", "unused"):
                    self.fail("Multi-dispositivo debe fallar")
            self.assertEqual(error.exception.code, 2)
            journal.assert_not_called()
            create.assert_not_called()


class ExistingHttpBoundaryTests(unittest.TestCase):
    def test_http_cannot_register_control_or_submit_hefesto(self):
        brain = Mock(cfg={"cors_origin": ""})
        handler = atlas_relay.make_handler(
            brain, ipaddress.ip_network("127.0.0.0/24"), {}, Mock())
        with ThreadingHTTPServer(("127.0.0.1", 0), handler) as server:
            thread = threading.Thread(target=server.serve_forever)
            thread.start()
            try:
                with patch.object(atlas_relay.hefesto, "Journal") as journal:
                    for route in ("register", "heartbeat", "control",
                                  "submit", "next", "result"):
                        conn = http.client.HTTPConnection(*server.server_address, timeout=3)
                        try:
                            conn.request("POST", f"/v1/hefesto/{route}",
                                         body='{"protocolVersion":2}',
                                         headers={"Content-Type": "application/json"})
                            response = conn.getresponse()
                            self.assertEqual(response.status, 404)
                            self.assertEqual(json.loads(response.read())["error"], "not_found")
                        finally:
                            conn.close()
                    journal.assert_not_called()
                    self.assertEqual(brain.method_calls, [])
            finally:
                server.shutdown()
                thread.join()


class AtlasRegressionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        with patch.dict("os.environ", {}, clear=True):
            cfg = atlas_relay.read_env()
        cfg["data_dir"] = temporary.name
        self.brain = atlas_relay.Brain(cfg)
        upstream = patch.object(atlas_relay.urllib.request, "urlopen",
                                side_effect=AssertionError("Unexpected upstream request"))
        upstream.start()
        self.addCleanup(upstream.stop)
        logging = patch.object(atlas_relay, "log")
        logging.start()
        self.addCleanup(logging.stop)

    def test_commands_keep_identity_without_provider_or_hefesto(self):
        for command in ("/help", "/ayuda", "/log", "/registro", "/unknown"):
            with self.subTest(command=command):
                ids = self.brain.receive(command, "local")
                snapshot = self.brain.get_snapshot()
                reply = snapshot["messages"][-1]
                self.assertEqual(reply["requestId"], ids["requestId"])
                self.assertEqual(reply["role"], "assistant")
                self.assertTrue(reply["content"])
                self.assertEqual(snapshot["state"], "completed")
                self.assertEqual(self.brain.jobs.qsize(), 0)
        self.assertEqual(self.brain.token_usage["calls"], 0)

    def test_clear_still_creates_a_new_session(self):
        before = self.brain.receive("/help", "local")
        after = self.brain.receive("/clear", "local")
        self.assertNotEqual(before["sessionId"], after["sessionId"])
        self.assertEqual(len(self.brain.get_snapshot()["messages"]), 1)

    def test_cancel_denies_pending_confirmation(self):
        ids = self.brain.receive("test cancellation", "local")
        event = threading.Event()
        confirmation = {"decision": None, "event": event}
        self.brain.confirmations[ids["requestId"]] = confirmation
        self.assertTrue(self.brain.cancel(ids["requestId"]))
        self.assertTrue(event.is_set())
        self.assertEqual(confirmation["decision"], "deny")
        self.assertEqual(self.brain.get_snapshot()["state"], "cancelled")
        self.assertNotIn(ids["requestId"], self.brain.confirmations)

    def test_llm_catalog_does_not_expose_hefesto(self):
        names = {tool["function"]["name"] for tool in atlas_relay.tool_payload()}
        self.assertEqual(names, {"calendar_read", "calendar_write"})

    def test_existing_routes_do_not_delegate_to_hefesto(self):
        handler = atlas_relay.make_handler(
            self.brain, ipaddress.ip_network("127.0.0.0/24"),
            {"telegram": "disabled", "llm": "stub", "calendar": "disabled"}, Mock())
        requests = [("/v1/health", None), ("/v1/session", None),
                    ("/v1/send", {"content": "/help", "toolId": "hefesto.echo"}),
                    ("/v1/confirm", {"requestId": "stale", "decision": "allow_once"}),
                    ("/v1/cancel", {"requestId": "stale"}),
                    ("/v1/system/cancel_all", {})]
        with ThreadingHTTPServer(("127.0.0.1", 0), handler) as server:
            thread = threading.Thread(target=server.serve_forever)
            thread.start()
            try:
                with patch.object(atlas_relay.hefesto.Journal, "dispatch",
                                  side_effect=AssertionError("HTTP must not delegate")) as dispatch:
                    for path, body in requests:
                        conn = http.client.HTTPConnection(*server.server_address, timeout=3)
                        try:
                            conn.request("GET" if body is None else "POST", path,
                                         body=None if body is None else json.dumps(body),
                                         headers={"Content-Type": "application/json"})
                            response = conn.getresponse()
                            self.assertEqual(response.status, 200, path)
                            response.read()
                        finally:
                            conn.close()
                    dispatch.assert_not_called()
            finally:
                server.shutdown()
                thread.join()


if __name__ == "__main__":
    unittest.main()
