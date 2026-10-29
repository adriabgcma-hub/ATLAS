"""Pruebas de delegación por intención del cerebro (entrega 3)."""

from __future__ import annotations

import json
import tempfile
import unittest

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
                return (404, {"protocolVersion": 1, "error": "invocation_not_found"})
            return (200, {"protocolVersion": 1,
                          "invocationId": body["invocationId"], **entry})
        return (404, {"protocolVersion": 1, "error": "not_found"})

    def accept(self, invocation_id="inv-1"):
        self.submit_results.append((200, {"protocolVersion": 1,
                                          "invocationId": invocation_id,
                                          "state": "pending"}))

    def reject(self, code):
        self.submit_results.append((409, {"protocolVersion": 1, "error": code}))


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
