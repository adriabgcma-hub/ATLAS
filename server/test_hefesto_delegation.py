"""Pruebas de delegación por intención del cerebro (entrega 3)."""

from __future__ import annotations

import json
import tempfile
import unittest
from unittest import mock

import atlas_relay


class FakeJournal:
    """Journal falso: respuestas de submit en cola y outcomes prefijados."""

    def __init__(self):
        self.submits = []
        self.submit_results = []
        self.outcomes = {}

    def dispatch(self, peer, method, path, raw):
        operation = path.rsplit("/", 1)[-1]
        body = json.loads(raw)
        if operation == "submit":
            self.submits.append(body)
            return self.submit_results.pop(0)
        if operation == "outcome":
            entry = self.outcomes.get(body["invocationId"])
            if entry is None:
                return (404, {"protocolVersion": 2, "error": "invocation_not_found"})
            return (200, {"protocolVersion": 2,
                          "invocationId": body["invocationId"], **entry})
        return (404, {"protocolVersion": 2, "error": "not_found"})

    def accept(self, invocation_id="inv-1"):
        self.submit_results.append((200, {"protocolVersion": 2,
                                          "invocationId": invocation_id,
                                          "state": "pending"}))

    def reject(self, code):
        self.submit_results.append((409, {"protocolVersion": 2, "error": code}))


class CancellingJournal(FakeJournal):
    """Journal que cancela el turno mientras se despacha el submit."""

    def __init__(self, brain):
        super().__init__()
        self.brain = brain

    def dispatch(self, peer, method, path, raw):
        self.brain.cancel("req-1")
        return super().dispatch(peer, method, path, raw)


class ControlJournal:
    """Journal falso para el control remoto: status (GET) y control (POST)."""

    def __init__(self, generation=3, suspended=False):
        self.generation = generation
        self.suspended = suspended
        self.controls = []

    def dispatch(self, peer, method, path, raw=b""):
        operation = path.rsplit("/", 1)[-1]
        if operation == "status":
            return (200, {"protocolVersion": 2, "deviceId": peer.device_id,
                          "generation": self.generation,
                          "suspended": self.suspended})
        if operation == "control":
            body = json.loads(raw)
            self.controls.append((body["action"], body["expectedGeneration"]))
            if body["expectedGeneration"] != self.generation:
                return (409, {"protocolVersion": 2, "error": "stale_generation"})
            self.generation += 1
            self.suspended = True
            return (200, {"protocolVersion": 2, "deviceId": peer.device_id,
                          "generation": self.generation, "suspended": True})
        return (404, {"protocolVersion": 2, "error": "not_found"})


class RacedControlJournal(ControlJournal):
    """Simula un abort ajeno: el primer control choca con la generación."""

    def __init__(self):
        super().__init__(generation=3)
        self.first_control = True

    def dispatch(self, peer, method, path, raw=b""):
        if path.endswith("control") and self.first_control:
            self.first_control = False
            body = json.loads(raw)
            self.controls.append((body["action"], body["expectedGeneration"]))
            self.generation += 1
            return (409, {"protocolVersion": 2, "error": "stale_generation"})
        return super().dispatch(peer, method, path, raw)


def make_brain(data_dir):
    cfg = {
        "token": "", "allowed": set(), "groq_keys": [],
        "groq_model": atlas_relay.DEFAULT_GROQ_MODEL,
        "groq_temperature": 0.7, "timezone": "Europe/Madrid",
        "caldav_user": "", "caldav_password": "",
        "editable": set(), "editable_names": [],
        "host": "127.0.0.1", "port": 8765,
        "data_dir": data_dir, "cors_origin": "",
    }
    return atlas_relay.Brain(cfg)


JOB = {"request_id": "req-1", "session_id": "ses-1", "source": "local",
       "chat_id": None, "deadline": 0.0}


class ToolAvailabilityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-delegation-")
        self.addCleanup(self.temp.cleanup)

    def test_sin_listener_la_herramienta_no_se_ofrece(self):
        brain = make_brain(self.temp.name)
        self.assertNotIn("hefesto_echo", [t["id"] for t in brain.tool_defs()])
        self.assertEqual(brain.execute_tool("hefesto_echo", {"text": "hola"}, JOB),
                         "Hefesto no está disponible (desactivado)")

    def test_con_listener_y_dispositivo_la_herramienta_se_ofrece(self):
        brain = make_brain(self.temp.name)
        brain._hefesto_journal = FakeJournal()
        brain._hefesto_device = "mac-one"
        self.assertIn("hefesto_echo", [t["id"] for t in brain.tool_defs()])
        payload = atlas_relay.tool_payload(brain.tool_defs())
        self.assertIn("hefesto_echo", [f["function"]["name"] for f in payload])

    def test_delegacion_aceptada_registra_la_entrada(self):
        brain = make_brain(self.temp.name)
        fake = FakeJournal()
        fake.accept()
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        result = brain.execute_tool("hefesto_echo", {"text": "hola"}, JOB)
        self.assertIn("delegado a Hefesto", result)
        entry = brain._delegations.get("inv-1")
        self.assertIsNotNone(entry)
        self.assertEqual(entry["request_id"], "req-1")
        self.assertEqual(entry["session_id"], "ses-1")
        submitted = fake.submits[0]
        self.assertEqual(submitted["toolId"], "hefesto.echo")
        self.assertEqual(submitted["arguments"], {"text": "hola"})
        self.assertEqual(submitted["requestId"], "req-1")

    def test_delegacion_rechazada_no_registra(self):
        brain = make_brain(self.temp.name)
        fake = FakeJournal()
        fake.reject("device_suspended")
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        result = brain.execute_tool("hefesto_echo", {"text": "hola"}, JOB)
        self.assertIn("Hefesto no está disponible (device_suspended)", result)
        self.assertEqual(brain._delegations, {})

    def test_texto_vacio_o_largo_se_rechaza_sin_encolar(self):
        brain = make_brain(self.temp.name)
        fake = FakeJournal()
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        self.assertIn("texto vacío",
                      brain.execute_tool("hefesto_echo", {"text": "  "}, JOB))
        self.assertIn("demasiado largo",
                      brain.execute_tool("hefesto_echo", {"text": "á" * 200}, JOB))
        self.assertEqual(fake.submits, [])


class ContinuationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-delegation-")
        self.addCleanup(self.temp.cleanup)

    def delegate(self, brain, fake, invocation_id="inv-1", job=None):
        fake.accept(invocation_id)
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        brain.execute_tool("hefesto_echo", {"text": "hola"}, job or JOB)
        return brain._delegations[invocation_id]

    def test_continuador_exitoso_con_etiqueta_y_task(self):
        brain = make_brain(self.temp.name)
        session_id = brain._active()["id"]
        fake = FakeJournal()
        entry = self.delegate(brain, fake, job={**JOB, "session_id": session_id})
        brain._hefesto_finish(entry, "succeeded", "hola mundo")
        last = brain.sessions[session_id]["messages"][-1]
        self.assertEqual(last["role"], "assistant")
        self.assertEqual(last["content"], "hola mundo")
        self.assertEqual(last["agent"], "hefesto")
        self.assertEqual(last["taskId"], entry["task_id"])
        event = brain.events[-1]
        self.assertEqual(event["type"], "conversation.message_appended")
        self.assertEqual(event["agent"], "hefesto")
        self.assertEqual(event["taskId"], entry["task_id"])
        self.assertNotEqual(event["requestId"], entry["request_id"])
        self.assertNotIn(entry["invocation_id"], brain._delegations)

    def test_continuador_avisa_por_telegram(self):
        brain = make_brain(self.temp.name)
        session_id = brain._active()["id"]
        fake = FakeJournal()
        entry = self.delegate(brain, fake,
                              job={**JOB, "session_id": session_id,
                                   "source": "telegram", "chat_id": 42})
        with mock.patch.object(brain, "_send_telegram") as send:
            brain._hefesto_finish(entry, "succeeded", "eco")
        send.assert_called_once_with(42, "Hefesto: eco")

    def test_cancelar_el_turno_descarta_la_delegacion(self):
        brain = make_brain(self.temp.name)
        session_id = brain._active()["id"]
        fake = FakeJournal()
        self.delegate(brain, fake, job={**JOB, "session_id": session_id})
        brain.cancel("req-1")
        self.assertEqual(brain._delegations, {})
        self.assertEqual(
            brain._load_delegations(), {})

    def test_cancelar_antes_de_la_herramienta_no_ejecuta(self):
        brain = make_brain(self.temp.name)
        fake = FakeJournal()
        fake.accept()
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        brain.cancel("req-1")
        result = brain._handle_tool_call(
            JOB, {"function": {"name": "hefesto_echo",
                               "arguments": json.dumps({"text": "hola"})}})
        self.assertEqual(result, "cancelada a mitad de turno")
        self.assertEqual(fake.submits, [])

    def test_cancelar_durante_el_envio_no_registra(self):
        brain = make_brain(self.temp.name)
        fake = CancellingJournal(brain)
        fake.accept()
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        result = brain.execute_tool("hefesto_echo", {"text": "hola"}, JOB)
        self.assertIn("cancelada", result)
        self.assertEqual(brain._delegations, {})

    def test_continuador_de_sesion_abandonada_se_descarta(self):
        brain = make_brain(self.temp.name)
        first = brain._active()["id"]
        fake = FakeJournal()
        entry = self.delegate(brain, fake, job={**JOB, "session_id": first})
        brain._local_reply("/clear", "local")
        brain._hefesto_finish(entry, "succeeded", "eco")
        self.assertNotIn("eco", [m["content"] for m in brain.sessions[first]["messages"]])
        self.assertNotIn(entry["invocation_id"], brain._delegations)

    def test_delegaciones_persisten_entre_instancias(self):
        brain1 = make_brain(self.temp.name)
        session_id = brain1._active()["id"]
        fake = FakeJournal()
        self.delegate(brain1, fake, job={**JOB, "session_id": session_id})
        brain2 = make_brain(self.temp.name)
        self.assertIn("inv-1", brain2._delegations)

    def test_load_delegaciones_descarta_entradas_incompletas(self):
        brain = make_brain(self.temp.name)
        with open(brain._delegations_path(), "w", encoding="utf-8") as fh:
            json.dump({"x": {"invocation_id": "x"}}, fh)
        self.assertEqual(brain._load_delegations(), {})


class ToolPhaseTests(unittest.TestCase):
    """El turno publica `executing` con el toolId antes de ejecutar."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-delegation-")
        self.addCleanup(self.temp.cleanup)

    def executing_events(self, brain):
        return [e for e in brain.events
                if e.get("kind") == "state" and e.get("state") == "executing"]

    def test_emite_executing_con_el_tool_antes_de_ejecutar(self):
        brain = make_brain(self.temp.name)
        fake = FakeJournal()
        fake.accept()
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        brain._handle_tool_call(
            JOB, {"function": {"name": "hefesto_echo",
                               "arguments": json.dumps({"text": "hola"})}})
        events = self.executing_events(brain)
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]["message"], "hefesto_echo")
        self.assertEqual(events[0]["requestId"], "req-1")
        self.assertEqual(len(fake.submits), 1)

    def test_denegar_no_emite_executing(self):
        brain = make_brain(self.temp.name)
        with mock.patch.object(brain, "ask_confirmation", return_value="deny"):
            brain._handle_tool_call(
                JOB, {"function": {"name": "hefesto_read_file",
                                   "arguments": json.dumps({"path": "/tmp/x"})}})
        self.assertEqual(self.executing_events(brain), [])

    def test_herramienta_desconocida_no_emite_executing(self):
        brain = make_brain(self.temp.name)
        brain._handle_tool_call(
            JOB, {"function": {"name": "nope", "arguments": "{}"}})
        self.assertEqual(self.executing_events(brain), [])


class TrackerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-delegation-")
        self.addCleanup(self.temp.cleanup)

    def delegate(self, brain, fake, invocation_id="inv-1"):
        fake.accept(invocation_id)
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        session_id = brain._active()["id"]
        brain.execute_tool("hefesto_echo", {"text": "hola"},
                           {**JOB, "session_id": session_id})
        return session_id

    def test_poll_resuelve_estados_terminales(self):
        brain = make_brain(self.temp.name)
        fake = FakeJournal()
        session_id = self.delegate(brain, fake)
        fake.outcomes["inv-1"] = {"state": "expired", "outputText": None}
        brain._hefesto_poll()
        last = brain.sessions[session_id]["messages"][-1]
        self.assertEqual(last["agent"], "hefesto")
        self.assertIn("caducó", last["content"])
        self.assertNotIn("inv-1", brain._delegations)

    def test_poll_no_toca_pendientes(self):
        brain = make_brain(self.temp.name)
        fake = FakeJournal()
        session_id = self.delegate(brain, fake)
        fake.outcomes["inv-1"] = {"state": "dispatched", "outputText": None}
        brain._hefesto_poll()
        self.assertIn("inv-1", brain._delegations)
        self.assertEqual(brain.sessions[session_id]["messages"], [])

    def test_poll_desconocido_y_cancelado_publican_continuador(self):
        for state, needle in (("unknown", "desconocido"), ("cancelled", "canceló")):
            with self.subTest(state=state):
                temp = tempfile.TemporaryDirectory(prefix="atlas-delegation-")
                self.addCleanup(temp.cleanup)
                brain = make_brain(temp.name)
                fake = FakeJournal()
                session_id = self.delegate(brain, fake)
                fake.outcomes["inv-1"] = {"state": state, "outputText": None}
                brain._hefesto_poll()
                last = brain.sessions[session_id]["messages"][-1]
                self.assertIn(needle, last["content"])

    def test_recuperacion_tras_reinicio_del_brain(self):
        brain1 = make_brain(self.temp.name)
        fake1 = FakeJournal()
        self.delegate(brain1, fake1)
        brain2 = make_brain(self.temp.name)
        fake2 = FakeJournal()
        brain2.hefesto_start(fake2, "mac-one")
        try:
            self.assertIn("inv-1", brain2._delegations)
            fake2.outcomes["inv-1"] = {"state": "unknown", "outputText": None}
            brain2._hefesto_poll()
            self.assertNotIn("inv-1", brain2._delegations)
        finally:
            brain2.hefesto_stop()

    def test_tracker_arranca_y_para_con_el_brain(self):
        brain = make_brain(self.temp.name)
        brain.hefesto_start(FakeJournal(), "mac-one")
        try:
            self.assertTrue(brain._hefesto_thread.is_alive())
        finally:
            brain.hefesto_stop()
        self.assertFalse(brain._hefesto_thread.is_alive())

    def test_sin_dispositivo_no_arranca_tracker(self):
        brain = make_brain(self.temp.name)
        brain.hefesto_start(FakeJournal(), None)
        self.assertIsNone(getattr(brain, "_hefesto_thread", None))
        self.assertNotIn("hefesto_echo", [t["id"] for t in brain.tool_defs()])


class RemoteControlTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-delegation-")
        self.addCleanup(self.temp.cleanup)

    def test_status_sin_listener(self):
        brain = make_brain(self.temp.name)
        self.assertEqual(brain.hefesto_status(),
                         {"configured": False, "deviceId": None,
                          "suspended": None, "generation": None})

    def test_status_con_listener(self):
        brain = make_brain(self.temp.name)
        brain._hefesto_journal = ControlJournal()
        brain._hefesto_device = "mac-one"
        self.assertEqual(brain.hefesto_status(),
                         {"configured": True, "deviceId": "mac-one",
                          "suspended": False, "generation": 3})

    def test_suspend_envia_abort_con_la_generacion_actual(self):
        brain = make_brain(self.temp.name)
        fake = ControlJournal()
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        self.assertEqual(brain.hefesto_suspend(), {"ok": True, "suspended": True})
        self.assertEqual(fake.controls, [("abort", 3)])

    def test_suspend_sin_listener(self):
        brain = make_brain(self.temp.name)
        self.assertEqual(brain.hefesto_suspend(),
                         {"ok": False, "error": "hefesto_disabled"})

    def test_suspend_reintenta_si_la_generacion_queda_obsoleta(self):
        brain = make_brain(self.temp.name)
        fake = RacedControlJournal()
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        self.assertEqual(brain.hefesto_suspend(), {"ok": True, "suspended": True})
        self.assertEqual(fake.controls, [("abort", 3), ("abort", 4)])


class ReadFlowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-read-")
        self.addCleanup(self.temp.cleanup)
        self.brain = make_brain(self.temp.name)
        self.brain._hefesto_journal = FakeJournal()
        self.brain._hefesto_device = "mac-one"

    def test_read_file_envia_submit_con_path_y_devuelve_el_contenido(self):
        journal = self.brain._hefesto_journal
        journal.accept("inv-9")
        journal.outcomes["inv-9"] = {"state": "succeeded", "outputText": "contenido"}
        result = self.brain.execute_tool(
            "hefesto_read_file", {"path": "/Users/x/Documents/a.txt"}, JOB)
        self.assertEqual(result, "contenido")
        self.assertEqual(journal.submits[0]["toolId"], "hefesto.read_file")
        self.assertEqual(journal.submits[0]["arguments"],
                         {"path": "/Users/x/Documents/a.txt"})

    def test_list_dir_envia_submit_con_el_toolid_correcto(self):
        journal = self.brain._hefesto_journal
        journal.accept("inv-7")
        journal.outcomes["inv-7"] = {"state": "succeeded", "outputText": "a.txt\nb"}
        result = self.brain.execute_tool(
            "hefesto_list_dir", {"path": "/Users/x/Documents"}, JOB)
        self.assertEqual(result, "a.txt\nb")
        self.assertEqual(journal.submits[0]["toolId"], "hefesto.list_dir")

    def test_read_con_ruta_relativa_no_envia_submit(self):
        result = self.brain.execute_tool("hefesto_read_file", {"path": "relativo"}, JOB)
        self.assertTrue(result.startswith("ruta inválida"))
        self.assertEqual(self.brain._hefesto_journal.submits, [])

    def test_read_con_submit_rechazado_devuelve_no_disponible(self):
        journal = self.brain._hefesto_journal
        journal.reject("device_suspended")
        result = self.brain.execute_tool(
            "hefesto_read_file", {"path": "/Users/x/Documents/a.txt"}, JOB)
        self.assertTrue(result.startswith("Hefesto no está disponible"))

    def test_read_sin_journal_devuelve_desactivado(self):
        self.brain._hefesto_journal = None
        self.brain._hefesto_device = None
        result = self.brain.execute_tool("hefesto_read_file", {"path": "/a"}, JOB)
        self.assertEqual(result, "Hefesto no está disponible (desactivado)")

    def test_catalogo_incluye_las_dos_lecturas_con_confirmacion(self):
        ids = {t["id"]: t for t in self.brain.tool_defs()}
        self.assertIn("hefesto_list_dir", ids)
        self.assertIn("hefesto_read_file", ids)
        self.assertTrue(ids["hefesto_read_file"]["requiresConfirmation"])
        self.assertTrue(ids["hefesto_list_dir"]["requiresConfirmation"])

    def test_confirm_summary_de_lectura_menciona_la_exportacion(self):
        summary, _ = self.brain._confirm_summary("hefesto_read_file", {"path": "/x/y.txt"})
        self.assertIn("Leer «/x/y.txt»", summary)
