"""Offline protocol, persistence, concurrency and ephemeral loopback mTLS tests."""

from __future__ import annotations

import concurrent.futures
import contextlib
import hashlib
import http.client
import io
import json
import shutil
import socket
import sqlite3
import ssl
import subprocess
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest import mock

try:
    from . import hefesto
except ImportError:
    import hefesto


PREFIX = "/v1/hefesto/"
DEVICE = hefesto.Peer("device", "mac-one")
CONTROLLER = hefesto.Peer("controller", "mac-one")


def encoded(**fields):
    return json.dumps({"protocolVersion": 3, **fields}, ensure_ascii=True).encode()


def submission(submission_id="submission-1", text="hello", **changes):
    return {"submissionId": submission_id, "requestId": "request-1", "sessionId": "session-1",
            "taskId": "task-1", "toolId": "hefesto.echo", "arguments": {"text": text}, **changes}


class Clock:
    def __init__(self):
        self.mono = 100.0
        self.wall = 1_800_000_000.0

    def advance(self, seconds):
        self.mono += seconds
        self.wall += seconds


class JournalTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="hefesto-journal-test-")
        self.addCleanup(self.temp.cleanup)
        self.path = str(Path(self.temp.name) / "journal.sqlite")
        self.clock = Clock()
        self.journal = self.new_journal()

    def new_journal(self):
        journal = hefesto.Journal(self.path, monotonic=lambda: self.clock.mono, wall_time=lambda: self.clock.wall)
        self.addCleanup(journal.close)
        return journal

    def call(self, operation, peer=DEVICE, **fields):
        return self.journal.dispatch(peer, "GET" if operation == "status" else "POST", PREFIX + operation,
                                     b"" if operation == "status" else encoded(**fields))

    def ok(self, operation, peer=DEVICE, **fields):
        status, value = self.call(operation, peer, **fields)
        self.assertEqual(status, 200, value)
        self.assertEqual(value["protocolVersion"], 3)
        return value

    def error(self, code, operation, peer=DEVICE, **fields):
        status, value = self.call(operation, peer, **fields)
        self.assertGreaterEqual(status, 400)
        self.assertEqual(value, {"protocolVersion": 3, "error": code})

    def ready(self, peer=DEVICE, instance="instance-1"):
        state = self.ok("register", peer, instanceId=instance)
        return self.ok("control", peer, action="resume", expectedGeneration=state["generation"])

    def send(self, submission_id="submission-1", text="hello", peer=CONTROLLER):
        return self.ok("submit", peer, **submission(submission_id, text))

    def next(self, peer=DEVICE):
        state = self.ok("status", peer)
        return self.ok("next", peer, instanceId=state["instanceId"], generation=state["generation"])["invocation"]

    @staticmethod
    def result_fields(invocation, **changes):
        return {"instanceId": invocation["instanceId"], "generation": invocation["generation"],
                "invocationId": invocation["invocationId"], "output": invocation["arguments"], **changes}

    def restart(self):
        self.journal.close()
        self.journal = self.new_journal()

    def test_full_echo_and_exact_response_contract(self):
        self.assertEqual(self.ok("status"), {"protocolVersion": 3, "deviceId": "mac-one", "instanceId": None,
                                           "generation": 0, "suspended": True, "online": False,
                                           "activeInvocationId": None})
        state = self.ready()
        self.assertEqual(state["generation"], 2)
        self.assertFalse(state["suspended"])
        self.assertTrue(state["online"])
        self.assertIsNone(self.next())
        ack = self.send(text="héllo 世界")
        self.assertEqual(set(ack), {"protocolVersion", "invocationId", "state"})
        self.assertEqual(ack["state"], "pending")
        self.assertEqual(self.ok("status")["activeInvocationId"], ack["invocationId"])
        invocation = self.next()
        self.assertEqual(invocation, {"protocolVersion": 3, "invocationId": ack["invocationId"],
                                     "requestId": "request-1", "sessionId": "session-1", "taskId": "task-1",
                                     "deviceId": "mac-one", "instanceId": "instance-1", "generation": 2,
                                     "toolId": "hefesto.echo", "arguments": {"text": "héllo 世界"},
                                     "expiresAtUnixMs": int(self.clock.wall * 1000) + 15000})
        self.assertEqual(self.next(), invocation)
        success = self.ok("result", **self.result_fields(invocation))
        self.assertEqual(success, {**ack, "state": "succeeded"})
        self.assertEqual(self.ok("result", **self.result_fields(invocation)), success)
        self.assertIsNone(self.ok("status")["activeInvocationId"])
        self.assertIsNone(self.next())
        self.assertEqual(self.send(text="héllo 世界"), success)
        self.assertEqual(self.send("submission-2")["state"], "pending")

    def test_no_queue_offline_suspended_or_busy(self):
        fields = submission()
        self.error("device_offline", "submit", CONTROLLER, **fields)
        self.error("device_offline", "control", action="resume", expectedGeneration=0)
        self.ok("register", instanceId="instance-1")
        self.error("device_suspended", "submit", CONTROLLER, **fields)
        self.error("device_suspended", "next", instanceId="instance-1", generation=1)
        self.ok("control", action="resume", expectedGeneration=1)
        self.assertIsNone(self.next())
        first = self.send()
        self.error("device_busy", "submit", CONTROLLER, **submission("submission-2"))
        self.assertEqual(self.send(), first)
        self.ok("control", action="abort", expectedGeneration=2)
        self.ok("control", action="resume", expectedGeneration=3)
        self.assertIsNone(self.next())
        self.assertEqual(self.send("submission-2")["state"], "pending")

    def test_submission_mutations_conflict_before_availability(self):
        self.ready()
        first = self.send()
        for changes in ({"requestId": "other"}, {"sessionId": "other"}, {"taskId": "other"},
                        {"arguments": {"text": "other"}}):
            with self.subTest(changes=changes):
                self.error("submission_conflict", "submit", CONTROLLER, **submission(**changes))
        self.clock.advance(15)
        self.assertEqual(self.send(), {**first, "state": "cancelled"})
        self.error("submission_conflict", "submit", CONTROLLER, **submission(text="mutated"))
        self.error("device_offline", "submit", CONTROLLER, **submission("new"))
        self.restart()
        self.assertEqual(self.send(), {**first, "state": "cancelled"})

    def test_submission_json_order_and_escaping_are_semantic(self):
        self.ready()
        first = self.send(text="é")
        fields = {"protocolVersion": 3, **submission(text="é")}
        raw = json.dumps(dict(reversed(list(fields.items()))), ensure_ascii=False, indent=2).encode()
        self.assertEqual(self.journal.dispatch(CONTROLLER, "POST", PREFIX + "submit", raw), (200, first))

    def test_duplicate_live_registration_does_not_resume_or_extend_lease(self):
        first = self.ok("register", instanceId="instance-1")
        self.clock.advance(14)
        self.assertEqual(self.ok("register", instanceId="instance-1"), first)
        self.clock.advance(1)
        self.error("instance_invalidated", "register", instanceId="instance-1")
        state = self.ok("status")
        self.assertEqual(state["generation"], 2)
        self.assertFalse(state["online"])
        self.assertTrue(state["suspended"])
        self.error("device_offline", "heartbeat", instanceId="instance-1")
        self.assertEqual(self.ok("status")["generation"], 2)
        self.restart()
        self.error("instance_invalidated", "register", instanceId="instance-1")
        state = self.ok("register", instanceId="instance-2")
        self.assertEqual(state["generation"], 3)
        self.assertTrue(state["suspended"])

    def test_heartbeat_extends_only_matching_instance_and_not_generation(self):
        self.ready()
        self.clock.advance(14)
        self.error("instance_mismatch", "heartbeat", instanceId="other")
        state = self.ok("heartbeat", instanceId="instance-1")
        self.assertEqual(state["generation"], 2)
        self.clock.advance(14)
        self.assertTrue(self.ok("status")["online"])
        self.clock.advance(1)
        self.assertFalse(self.ok("status")["online"])
        self.assertEqual(self.ok("status")["generation"], 3)

    def test_active_registration_cannot_be_replaced(self):
        self.ready()
        self.error("instance_conflict", "register", instanceId="instance-2")
        self.assertFalse(self.ok("register", instanceId="instance-1")["suspended"])
        self.clock.advance(15)
        self.assertTrue(self.ok("register", instanceId="instance-2")["suspended"])

    def test_simultaneous_registrations_have_one_winner(self):
        barrier = threading.Barrier(12)

        def register(index):
            barrier.wait(timeout=5)
            return self.call("register", instanceId=f"instance-{index}")

        with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
            results = list(pool.map(register, range(12)))
        self.assertEqual(sum(status == 200 for status, _ in results), 1)
        self.assertEqual(sum(value.get("error") == "instance_conflict" for _, value in results), 11)
        self.assertEqual(self.ok("status")["generation"], 1)

    def test_simultaneous_submissions_share_one_durable_invocation(self):
        self.ready()
        barrier = threading.Barrier(12)

        def submit(_index):
            barrier.wait(timeout=5)
            return self.call("submit", CONTROLLER, **submission())

        with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
            results = list(pool.map(submit, range(12)))
        self.assertTrue(all(result == results[0] for result in results))
        self.assertEqual(results[0][0], 200)
        with contextlib.closing(sqlite3.connect(self.path)) as db:
            self.assertEqual(db.execute("SELECT count(*) FROM invocations").fetchone()[0], 1)

    def test_roles_for_all_restricted_operations(self):
        self.ready()
        ack = self.send()
        requests = {"register": {"instanceId": "new"}, "heartbeat": {"instanceId": "instance-1"},
                    "next": {"instanceId": "instance-1", "generation": 2},
                    "result": {"instanceId": "instance-1", "generation": 2,
                               "invocationId": ack["invocationId"], "output": {"text": "hello"}}}
        for operation, fields in requests.items():
            with self.subTest(operation=operation):
                self.error("forbidden", operation, CONTROLLER, **fields)
        self.error("forbidden", "submit", DEVICE, **submission())
        self.assertEqual(self.ok("status", DEVICE), self.ok("status", CONTROLLER))
        state = self.ok("control", CONTROLLER, action="abort", expectedGeneration=2)
        self.assertTrue(state["suspended"])
        self.assertEqual(self.journal.dispatch(None, "GET", PREFIX + "status"),
                         (403, {"protocolVersion": 3, "error": "unauthorized_peer"}))

    def test_device_identity_isolated_including_idempotency(self):
        other_device = hefesto.Peer("device", "mac-two")
        other_controller = hefesto.Peer("controller", "mac-two")
        self.ready()
        self.ready(other_device)
        first = self.send()
        invocation = self.next()
        second = self.send(peer=other_controller)
        self.assertNotEqual(first["invocationId"], second["invocationId"])
        self.error("invocation_not_found", "result", other_device, **self.result_fields(invocation))
        self.ok("control", other_controller, action="abort", expectedGeneration=2)
        self.assertFalse(self.ok("status")["suspended"])
        self.assertEqual(self.next(), invocation)

    def test_wrong_instance_and_generation_never_dispatch_or_complete(self):
        self.ready()
        self.send()
        for fields, code in (({"instanceId": "wrong", "generation": 2}, "instance_mismatch"),
                             ({"instanceId": "instance-1", "generation": 1}, "stale_generation")):
            self.error(code, "next", **fields)
        self.assertEqual(self.send()["state"], "pending")
        invocation = self.next()
        self.error("instance_mismatch", "result", **self.result_fields(invocation, instanceId="wrong"))
        self.error("stale_generation", "result", **self.result_fields(invocation, generation=1))
        self.error("result_conflict", "result", **self.result_fields(invocation, output={"text": "forged"}))
        self.assertEqual(self.send()["state"], "dispatched")
        self.assertEqual(self.ok("result", **self.result_fields(invocation))["state"], "succeeded")

    def test_result_before_dispatch_is_rejected(self):
        self.ready()
        ack = self.send()
        self.error("invocation_not_dispatched", "result", instanceId="instance-1", generation=2,
                   invocationId=ack["invocationId"], output={"text": "hello"})
        self.assertEqual(self.send(), ack)

    def test_success_survives_lost_ack_expiry_abort_restart_and_new_instance(self):
        self.ready()
        self.send()
        invocation = self.next()
        success = self.ok("result", **self.result_fields(invocation))
        self.ok("control", action="abort", expectedGeneration=2)
        self.clock.advance(30)
        self.restart()
        self.ready(instance="new-instance")
        self.assertEqual(self.ok("result", **self.result_fields(invocation)), success)
        self.assertEqual(self.send(), success)
        for changes, code in (({"output": {"text": "changed"}}, "result_conflict"),
                              ({"generation": 99}, "stale_generation"),
                              ({"instanceId": "new-instance"}, "instance_mismatch")):
            self.error(code, "result", **self.result_fields(invocation, **changes))
            self.assertEqual(self.send(), success)

    def test_abort_cancels_pending_and_resume_never_recovers_it(self):
        self.ready()
        first = self.send()
        self.error("stale_generation", "control", action="abort", expectedGeneration=1)
        state = self.ok("control", action="abort", expectedGeneration=2)
        self.assertEqual(state["generation"], 3)
        self.assertTrue(state["suspended"])
        self.assertIsNone(state["activeInvocationId"])
        self.assertEqual(self.send(), {**first, "state": "cancelled"})
        self.error("stale_generation", "control", action="resume", expectedGeneration=2)
        self.ok("control", action="resume", expectedGeneration=3)
        self.assertIsNone(self.next())
        self.assertEqual(self.send(), {**first, "state": "cancelled"})

    def test_abort_dispatched_is_unknown_and_late_result_cannot_overwrite(self):
        self.ready()
        self.send()
        invocation = self.next()
        self.ok("control", action="abort", expectedGeneration=2)
        self.error("invocation_not_dispatched", "result", **self.result_fields(invocation))
        self.assertEqual(self.send()["state"], "unknown")
        self.ok("control", action="resume", expectedGeneration=3)
        self.assertIsNone(self.next())
        self.error("invocation_not_dispatched", "result", **self.result_fields(invocation))

    def test_abort_while_suspended_or_offline_increments_and_stays_suspended(self):
        state = self.ok("control", CONTROLLER, action="abort", expectedGeneration=0)
        self.assertEqual(state["generation"], 1)
        self.assertTrue(state["suspended"])
        self.assertFalse(state["online"])
        self.ok("register", instanceId="instance-1")
        state = self.ok("control", action="abort", expectedGeneration=2)
        self.assertEqual(state["generation"], 3)
        self.assertTrue(state["suspended"])
        self.ok("control", action="resume", expectedGeneration=3)
        self.assertEqual(self.ok("control", action="resume", expectedGeneration=4)["generation"], 4)

    def test_lease_expiry_cancels_pending_and_invalidates_generation(self):
        self.ready()
        self.send()
        self.clock.advance(15)
        self.error("stale_generation", "control", action="resume", expectedGeneration=2)
        self.assertEqual(self.send()["state"], "cancelled")
        self.assertEqual(self.ok("status")["generation"], 3)
        self.error("instance_invalidated", "register", instanceId="instance-1")

    def test_lease_expiry_dispatched_unknown_never_retried(self):
        self.ready()
        self.send()
        invocation = self.next()
        self.clock.advance(15)
        self.error("invocation_not_dispatched", "result", **self.result_fields(invocation))
        self.assertEqual(self.send()["state"], "unknown")
        self.ready(instance="new")
        self.assertIsNone(self.next())
        self.assertEqual(self.send()["state"], "unknown")

    def test_invocation_monotonic_deadline_despite_backwards_wall_clock(self):
        self.ready()
        self.send()
        self.clock.advance(10)
        self.ok("heartbeat", instanceId="instance-1")
        self.clock.wall -= 5000
        self.clock.mono += 5
        self.assertIsNone(self.next())
        self.assertEqual(self.send()["state"], "expired")
        self.assertTrue(self.ok("status")["online"])
        self.assertEqual(self.ok("status")["generation"], 2)
        self.assertEqual(self.send("new")["state"], "pending")

    def test_lease_monotonic_deadline_despite_backwards_wall_clock(self):
        self.ready()
        self.clock.wall -= 5000
        self.clock.mono += 15
        self.assertFalse(self.ok("status")["online"])
        self.error("device_offline", "heartbeat", instanceId="instance-1")

    def test_wall_ttl_is_independent_of_monotonic_deadline(self):
        self.ready()
        self.send()
        invocation = self.next()
        self.clock.wall += 15
        self.error("invocation_not_dispatched", "result", **self.result_fields(invocation))
        self.assertEqual(self.send()["state"], "unknown")
        self.assertTrue(self.ok("status")["online"])
        self.clock.wall -= 15
        self.assertEqual(self.send()["state"], "unknown")
        self.assertIsNone(self.next())

    def test_pending_wall_expiry_and_dispatched_monotonic_expiry(self):
        self.ready()
        self.send()
        self.clock.wall += 15
        self.assertEqual(self.send()["state"], "expired")
        self.send("second")
        invocation = self.next()
        self.clock.advance(10)
        self.ok("heartbeat", instanceId="instance-1")
        self.clock.wall -= 5000
        self.clock.mono += 5
        self.error("invocation_not_dispatched", "result", **self.result_fields(invocation))
        self.assertEqual(self.send("second")["state"], "unknown")

    def test_restart_cancels_pending_and_invalidates_live_instance(self):
        self.ready()
        first = self.send()
        self.restart()
        self.assertEqual(self.send(), {**first, "state": "cancelled"})
        state = self.ok("status")
        self.assertFalse(state["online"])
        self.assertTrue(state["suspended"])
        self.assertEqual(state["generation"], 3)
        self.error("instance_invalidated", "register", instanceId="instance-1")
        self.error("device_offline", "heartbeat", instanceId="instance-1")
        self.ready(instance="new")
        self.assertIsNone(self.next())
        self.restart()
        self.error("instance_invalidated", "register", instanceId="new")
        self.error("instance_invalidated", "register", instanceId="instance-1")

    def test_restart_dispatched_unknown_even_with_monotonic_epoch_change(self):
        self.ready()
        self.send()
        invocation = self.next()
        self.clock.mono = 0
        self.clock.wall -= 5000
        self.restart()
        self.assertEqual(self.send()["state"], "unknown")
        self.error("invocation_not_dispatched", "result", **self.result_fields(invocation))
        self.ready(instance="fresh")
        self.assertIsNone(self.next())
        self.assertEqual(self.send()["state"], "unknown")

    def test_every_operation_sweeps_even_rejected_requests(self):
        for operation, fields in (("status", {}), ("register", {"instanceId": "old"}),
                                  ("heartbeat", {"instanceId": "old"}),
                                  ("control", {"action": "resume", "expectedGeneration": 2}),
                                  ("submit", submission()), ("next", {"instanceId": "old", "generation": 2}),
                                  ("result", {"instanceId": "old", "generation": 2,
                                              "invocationId": "missing", "output": {"text": "hello"}})):
            with self.subTest(operation=operation):
                peer = hefesto.Peer("device", "sweep-" + operation)
                controller = hefesto.Peer("controller", peer.device_id)
                self.ready(peer, instance="old")
                self.send(peer=controller)
                self.clock.advance(15)
                self.call(operation, controller if operation == "submit" else peer, **fields)
                self.assertEqual(self.ok("status", peer)["generation"], 3)
                self.assertEqual(self.send(peer=controller)["state"], "cancelled")
        self.ready()
        self.clock.advance(15)
        self.journal.dispatch(DEVICE, "POST", PREFIX + "register", b"not-json")
        with contextlib.closing(sqlite3.connect(self.path)) as db:
            self.assertEqual(db.execute("SELECT instance_id,generation FROM devices WHERE device_id='mac-one'").fetchone(),
                             (None, 3))

    def test_result_abort_race_never_overwrites_success_or_unknown(self):
        self.ready()
        self.send()
        invocation = self.next()
        barrier = threading.Barrier(2)

        def result():
            barrier.wait(timeout=5)
            return self.call("result", **self.result_fields(invocation))

        def abort():
            barrier.wait(timeout=5)
            return self.call("control", action="abort", expectedGeneration=2)

        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            result_future, abort_future = pool.submit(result), pool.submit(abort)
            result_status, _ = result_future.result(timeout=5)
            self.assertEqual(abort_future.result(timeout=5)[0], 200)
        final_state = self.send()["state"]
        self.assertEqual(final_state, "succeeded" if result_status == 200 else "unknown")
        self.assertTrue(self.ok("status")["suspended"])

    def test_exact_keys_types_versions_and_echo_policy(self):
        valid = {"register": {"instanceId": "i"}, "heartbeat": {"instanceId": "i"},
                 "control": {"action": "resume", "expectedGeneration": 0}, "submit": submission(),
                 "next": {"instanceId": "i", "generation": 0},
                 "result": {"instanceId": "i", "generation": 0, "invocationId": "v", "output": {"text": "hi"}}}
        for operation, fields in valid.items():
            peer = CONTROLLER if operation == "submit" else DEVICE
            for extra in ("deviceId", "source", "risk", "agent", "unknown"):
                with self.subTest(operation=operation, extra=extra):
                    self.error("invalid_fields", operation, peer, **fields, **{extra: "spoof"})
            for key in fields:
                with self.subTest(operation=operation, missing=key):
                    missing = dict(fields)
                    del missing[key]
                    self.error("invalid_fields", operation, peer, **missing)
            for version in (True, False, 1.0, "1", 0, 1, 2, None, [], {}):
                with self.subTest(operation=operation, version=version):
                    self.error("invalid_version", operation, peer, **fields, protocolVersion=version)
        for value in (True, 1.0, "1", -1, 2**63, None, [], {}):
            self.error("invalid_generation", "next", instanceId="i", generation=value)
            self.error("invalid_generation", "control", action="resume", expectedGeneration=value)
            self.error("invalid_generation", "result", instanceId="i", generation=value,
                       invocationId="v", output={"text": "hello"})
        for value in ("shell", "hefesto.exec", {}, [], 1, True, None):
            self.error("unsupported_tool", "submit", CONTROLLER, **submission(toolId=value))
        for value in ("restart", "RESUME", "", [], {}, None, True):
            self.error("invalid_action", "control", action=value, expectedGeneration=0)

    def test_ids_have_exact_ascii_and_length_bounds(self):
        invalid = ("", "a" * 81, "a b", "é", "a/b", "a.b", "a\n", "a\x00", True, 1, None, [], {})
        for value in invalid:
            with self.subTest(value=value):
                self.error("invalid_id", "register", instanceId=value)
                self.error("invalid_id", "heartbeat", instanceId=value)
                for key in ("submissionId", "requestId", "sessionId", "taskId"):
                    self.error("invalid_id", "submit", CONTROLLER, **submission(**{key: value}))
                self.error("invalid_id", "result", instanceId="i", generation=0,
                           invocationId=value, output={"text": "hello"})
        self.ok("register", instanceId="_-aZ09" + "a" * 74)

    def test_text_limits_utf8_and_surrogates(self):
        self.ready()
        for index, text in enumerate(("", "a" * 256, "é" * 128, "\U0001f600" * 64, "\x00\n\r")):
            with self.subTest(length=len(text)):
                self.send(str(index), text)
                invocation = self.next()
                self.ok("result", **self.result_fields(invocation))
        for text in ("a" * 257, "é" * 129, "\U0001f600" * 65, "\ud800", 1, True, None, [], {}):
            with self.subTest(text_type=type(text)):
                self.error("invalid_text", "submit", CONTROLLER, **submission(text=text))
        for text, code in (("\ud800", "invalid_text"), (1, "invalid_output"), (True, "invalid_output"),
                           (None, "invalid_output"), ([], "invalid_output"), ({}, "invalid_output")):
            with self.subTest(output_type=type(text)):
                self.error(code, "result", instanceId="i", generation=2, invocationId="v",
                           output={"text": text})
        for arguments in ({}, {"text": "hi", "extra": 1}):
            self.error("invalid_text", "submit", CONTROLLER, **submission(arguments=arguments))
        for arguments in ([], None, "text"):
            self.error("invalid_fields", "submit", CONTROLLER, **submission(arguments=arguments))

    def test_raw_json_duplicates_invalid_encoding_and_body_limit(self):
        cases = (b'{"protocolVersion":3,"protocolVersion":3,"instanceId":"i"}',
                 b'{"protocolVersion":3,"instanceId":"i","instanceId":"i"}',
                 b'{"protocolVersion":NaN,"instanceId":"i"}',
                 b'{"protocolVersion":Infinity,"instanceId":"i"}', b"\xff", b"", b"{}{}")
        for raw in cases:
            with self.subTest(raw=raw[:60]):
                self.assertEqual(self.journal.dispatch(DEVICE, "POST", PREFIX + "register", raw),
                                 (400, {"protocolVersion": 3, "error": "invalid_json"}))
        self.assertEqual(self.journal.dispatch(DEVICE, "POST", PREFIX + "register", b"[" * 2000 + b"]" * 2000)[0], 400)
        raw = encoded(**submission()).replace(b'"text": "hello"', b'"text":"hello","text":"hello"')
        self.assertEqual(self.journal.dispatch(CONTROLLER, "POST", PREFIX + "submit", raw)[1]["error"], "invalid_json")
        for raw in (b"[]", b"null", b"true", b'"hello"', b"1", b"{}"):
            self.assertEqual(self.journal.dispatch(DEVICE, "POST", PREFIX + "register", raw)[1]["error"], "invalid_fields")
        self.assertEqual(self.journal.dispatch(DEVICE, "POST", PREFIX + "register",
                                               b"x" * (hefesto.MAX_BODY_BYTES + 1))[0], 413)
        raw = encoded(instanceId="i")
        self.assertEqual(self.journal.dispatch(DEVICE, "POST", PREFIX + "register",
                                               raw + b" " * (hefesto.MAX_BODY_BYTES - len(raw)))[0], 200)
        self.assertEqual(self.journal.dispatch(DEVICE, "POST", PREFIX + "register", {})[1]["error"], "invalid_body")

    def test_paths_methods_status_body_and_no_authority_from_json(self):
        for path in (PREFIX + "status?x=1", PREFIX + "status/", PREFIX + "../status", "/v1/status", "https://host" + PREFIX + "status"):
            self.assertEqual(self.journal.dispatch(DEVICE, "GET", path)[0], 404)
        self.assertEqual(self.journal.dispatch(DEVICE, "POST", PREFIX + "status", b"{}")[0], 405)
        self.assertEqual(self.journal.dispatch(DEVICE, "GET", PREFIX + "register")[0], 405)
        self.assertEqual(self.journal.dispatch(DEVICE, "GET", PREFIX + "status", b"{}")[1]["error"], "invalid_body")
        self.error("invalid_fields", "register", instanceId="i", deviceId="mac-one")

    def test_failures_are_sanitized_and_close_is_idempotent(self):
        with mock.patch.object(self.journal, "_monotonic", side_effect=RuntimeError("private /credentials/key.pem")):
            self.assertEqual(self.call("status"), (503, {"protocolVersion": 3, "error": "journal_unavailable"}))
        self.assertFalse(self.ok("status")["online"])
        self.journal.close()
        self.journal.close()
        self.assertEqual(self.call("status")[1]["error"], "journal_unavailable")
        with self.assertRaisesRegex(ValueError, "^journal_unavailable$"):
            hefesto.Journal(str(Path(self.temp.name) / "missing" / "sensitive.sqlite"))

    def test_outcome_exige_rol_controller(self):
        self.ready()
        self.send()
        self.error("forbidden", "outcome", peer=DEVICE, invocationId="inv-1")

    def test_outcome_devuelve_estado_y_resultado(self):
        self.ready()
        ack = self.send(text="héllo 世界")
        outcome = self.ok("outcome", peer=CONTROLLER, invocationId=ack["invocationId"])
        self.assertEqual(outcome, {"protocolVersion": 3, "invocationId": ack["invocationId"],
                                   "state": "pending", "output": None})
        invocation = self.next()
        self.ok("result", peer=DEVICE, **self.result_fields(invocation))
        outcome = self.ok("outcome", peer=CONTROLLER, invocationId=ack["invocationId"])
        self.assertEqual(outcome["state"], "succeeded")
        self.assertEqual(outcome["output"], {"text": "héllo 世界"})

    def test_outcome_desconocido_devuelve_404(self):
        self.error("invocation_not_found", "outcome", peer=CONTROLLER, invocationId="no-existe")

    def test_outcome_exige_invocation_id(self):
        self.error("invalid_fields", "outcome", peer=CONTROLLER)

    def test_outcome_es_lectura_pura(self):
        self.ready()
        ack = self.send()
        self.next()
        self.ok("outcome", peer=CONTROLLER, invocationId=ack["invocationId"])
        self.assertEqual(self.ok("status")["activeInvocationId"], ack["invocationId"])


class ReadToolTests(JournalTests):
    def test_submit_read_file_aceptado(self):
        self.ready()
        ack = self.ok("submit", CONTROLLER, **submission(
            toolId="hefesto.read_file", arguments={"path": "/Users/x/Documents/a.txt"}))
        self.assertEqual(set(ack), {"protocolVersion", "invocationId", "state"})
        self.assertEqual(ack["state"], "pending")

    def test_submit_list_dir_aceptado(self):
        self.ready()
        ack = self.ok("submit", CONTROLLER, **submission(
            toolId="hefesto.list_dir", arguments={"path": "/Users/x/Documents"}))
        self.assertEqual(ack["state"], "pending")

    def test_submit_con_tool_desconocido_rechazado(self):
        self.ready()
        self.error("unsupported_tool", "submit", CONTROLLER,
                   **submission(toolId="hefesto.mkdir", arguments={"path": "/x"}))

    def test_read_con_argumento_text_rechazado(self):
        self.ready()
        # toolId de lectura con arguments {text} (shape de eco) => invalid_fields
        self.error("invalid_fields", "submit", CONTROLLER,
                   **submission(toolId="hefesto.read_file"))

    def test_read_con_ruta_relativa_rechazada(self):
        self.ready()
        self.error("invalid_path", "submit", CONTROLLER,
                   **submission(toolId="hefesto.read_file", arguments={"path": "relativo"}))

    def test_next_devuelve_toolid_y_arguments_del_submit(self):
        self.ready()
        ack = self.ok("submit", CONTROLLER, **submission(
            toolId="hefesto.read_file", arguments={"path": "/Users/x/Documents/a.txt"}))
        invocation = self.next()
        self.assertEqual(invocation["invocationId"], ack["invocationId"])
        self.assertEqual(invocation["toolId"], "hefesto.read_file")
        self.assertEqual(invocation["arguments"], {"path": "/Users/x/Documents/a.txt"})

    def test_result_de_lectura_acepta_texto_libre(self):
        self.ready()
        self.ok("submit", CONTROLLER, **submission(
            toolId="hefesto.read_file", arguments={"path": "/Users/x/Documents/a.txt"}))
        invocation = self.next()
        done = self.ok("result", **self.result_fields(invocation, output={"text": "contenido leído"}))
        self.assertEqual(done["state"], "succeeded")

    def test_result_de_eco_sigue_exigiendo_eco_exacto(self):
        self.ready()
        self.send(text="hola")
        invocation = self.next()
        self.error("result_conflict", "result",
                   **self.result_fields(invocation, output={"text": "distinto"}))


WRITE = {"submissionId": "submission-w", "requestId": "request-1", "sessionId": "session-1",
         "taskId": "task-1", "toolId": "hefesto.write_file",
         "arguments": {"path": "/Users/x/Documents/n.txt", "content": "hola",
                       "expectedHash": None}}
RENAME = {"submissionId": "submission-r", "requestId": "request-1", "sessionId": "session-1",
          "taskId": "task-2", "toolId": "hefesto.rename",
          "arguments": {"from": "/Users/x/Documents/a.txt", "to": "/Users/x/Documents/b.txt"}}
INFO = {"submissionId": "submission-i", "requestId": "request-1", "sessionId": "session-1",
        "taskId": "task-3", "toolId": "hefesto.file_info",
        "arguments": {"path": "/Users/x/Documents/a.txt"}}


class WriteJournalTests(JournalTests):
    def test_el_contenido_no_se_guarda_en_sqlite(self):
        self.ready()
        self.ok("submit", peer=CONTROLLER, **WRITE)
        invocation = self.next()
        self.assertEqual(invocation["arguments"]["content"], "hola")
        with sqlite3.connect(self.path) as db:
            raw = db.execute("SELECT submission FROM invocations").fetchone()[0]
        self.assertNotIn("hola", raw)
        self.assertIn("contentSha256", raw)

    def test_reinicio_sin_contenido_deja_unknown_y_no_reenvia(self):
        self.ready()
        self.ok("submit", peer=CONTROLLER, **WRITE)
        self.next()
        self.restart()
        self.ready(instance="instance-2")
        self.assertIsNone(self.next())

    def test_write_valida_argumentos_y_salida(self):
        self.ready()
        broken = json.loads(json.dumps(WRITE))
        broken["arguments"]["expectedHash"] = "XYZ"
        self.error("invalid_hash", "submit", peer=CONTROLLER, **broken)
        self.ok("submit", peer=CONTROLLER, **WRITE)
        invocation = self.next()
        value = self.ok(
            "result",
            **self.result_fields(invocation, output={"text": "Escrito «x» (4 B, sha256 abc)"}))
        self.assertEqual(value["state"], "succeeded")

    def test_file_info_admite_info_o_texto_de_rechazo(self):
        self.ready()
        self.ok("submit", peer=CONTROLLER, **INFO)
        invocation = self.next()
        self.ok("result",
                **self.result_fields(invocation, output={"exists": True, "size": 3,
                                                         "sha256": "0" * 64}))
        value = self.ok("outcome", peer=CONTROLLER,
                        invocationId=invocation["invocationId"])
        self.assertEqual(value["output"]["size"], 3)
        self.ok("submit", peer=CONTROLLER,
                **{**INFO, "submissionId": "submission-i2", "taskId": "task-4"})
        rejection = self.next()
        self.ok("result", **self.result_fields(
            rejection, output={"text": "fuera de las raíces autorizadas"}))

    def test_write_rechaza_la_salida_de_info(self):
        self.ready()
        self.ok("submit", peer=CONTROLLER, **WRITE)
        invocation = self.next()
        self.error("invalid_text", "result",
                   **self.result_fields(invocation, output={"exists": True, "size": 3,
                                                            "sha256": "0" * 64}))

    def test_submit_idempotente_no_duplica_y_conflicta_si_cambia_el_contenido(self):
        self.ready()
        first = self.ok("submit", peer=CONTROLLER, **WRITE)
        self.assertEqual(self.ok("submit", peer=CONTROLLER, **WRITE), first)
        changed = json.loads(json.dumps(WRITE))
        changed["arguments"]["content"] = "otro"
        self.error("submission_conflict", "submit", peer=CONTROLLER, **changed)

    def test_rename_valida_rutas(self):
        self.ready()
        broken = json.loads(json.dumps(RENAME))
        broken["arguments"]["to"] = "relativo"
        self.error("invalid_path", "submit", peer=CONTROLLER, **broken)
        self.ok("submit", peer=CONTROLLER, **RENAME)


class RegistryTests(unittest.TestCase):
    def test_peer_validation(self):
        for role, device in (("admin", "mac"), (None, "mac"), ([], "mac"), ("device", ""),
                             ("device", "a" * 81), ("device", "é"), ("controller", True)):
            with self.subTest(role=role, device=device):
                with self.assertRaisesRegex(ValueError, "^invalid_peer$"):
                    hefesto.Peer(role, device)

    def test_registry_exact_validation_and_sanitized_errors(self):
        fingerprint = "ab" * 32
        with tempfile.TemporaryDirectory(prefix="hefesto-registry-test-") as directory:
            path = Path(directory) / "peers.json"
            path.write_text(json.dumps({fingerprint: {"role": "device", "deviceId": "mac-one"}}))
            self.assertEqual(hefesto.load_peers(path), {fingerprint: DEVICE})
            invalid = (["not-an-object"], {fingerprint.upper(): {"role": "device", "deviceId": "mac"}},
                       {"short": {"role": "device", "deviceId": "mac"}},
                       {fingerprint: {"role": "admin", "deviceId": "mac"}},
                       {fingerprint: {"role": "device", "deviceId": "mac", "extra": 1}},
                       {fingerprint: {"role": "device"}}, {fingerprint: None},
                       {fingerprint: {"role": "controller", "deviceId": "a/b"}})
            for value in invalid:
                path.write_text(json.dumps(value))
                with self.assertRaisesRegex(ValueError, "^invalid_peer_registry$"):
                    hefesto.load_peers(path)
            for raw in (b"not-json", b"\xff", ('{"' + fingerprint + '":{},"' + fingerprint + '":{}}').encode(),
                        ('{"' + fingerprint + '":{"role":"device","role":"device","deviceId":"mac"}}').encode()):
                path.write_bytes(raw)
                with self.assertRaisesRegex(ValueError, "^invalid_peer_registry$"):
                    hefesto.load_peers(path)
            with self.assertRaisesRegex(ValueError, "^invalid_peer_registry$"):
                hefesto.load_peers(Path(directory) / "missing-private-name")


class TLSFixture:
    def __init__(self):
        self.temp = tempfile.TemporaryDirectory(prefix="hefesto-tls-test-")
        self.root = Path(self.temp.name)
        try:
            self.make_ca("ca")
            self.make_ca("foreign-ca")
            for name in ("server", "device", "controller", "unlisted"):
                self.make_leaf(name, "ca")
            self.make_leaf("foreign", "foreign-ca")
        except BaseException:
            self.close()
            raise

    def file(self, name, suffix):
        return str(self.root / (name + suffix))

    def openssl(self, *args):
        subprocess.run(["openssl", *args], check=True, timeout=20,
                       stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def make_ca(self, name):
        config = self.root / (name + ".cnf")
        config.write_text("[req]\ndistinguished_name=dn\nx509_extensions=ca\nprompt=no\n"
                          f"[dn]\nCN=Hefesto test {name}\n[ca]\nbasicConstraints=critical,CA:TRUE\n"
                          "keyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\n"
                          "authorityKeyIdentifier=keyid:always\n")
        self.openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-sha256",
                     "-config", str(config), "-keyout", self.file(name, ".key"), "-out", self.file(name, ".pem"))

    def make_leaf(self, name, ca):
        config = self.root / (name + ".cnf")
        config.write_text("[req]\ndistinguished_name=dn\nprompt=no\n[dn]\nCN=localhost\n")
        extension = self.root / (name + ".ext")
        extension.write_text("basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\n"
                             "subjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid:always\n"
                             + ("extendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n"
                                if name == "server" else "extendedKeyUsage=clientAuth\n"))
        self.openssl("req", "-new", "-newkey", "rsa:2048", "-nodes", "-sha256", "-config", str(config),
                     "-keyout", self.file(name, ".key"), "-out", self.file(name, ".csr"))
        self.openssl("x509", "-req", "-in", self.file(name, ".csr"), "-CA", self.file(ca, ".pem"),
                     "-CAkey", self.file(ca, ".key"), "-set_serial", str(10 + len(list(self.root.glob("*.pem")))),
                     "-days", "1", "-sha256", "-extfile", str(extension), "-out", self.file(name, ".pem"))

    def fingerprint(self, name):
        return hashlib.sha256(ssl.PEM_cert_to_DER_cert(Path(self.file(name, ".pem")).read_text())).hexdigest()

    def context(self, name=None):
        context = ssl.create_default_context(ssl.Purpose.SERVER_AUTH, cafile=self.file("ca", ".pem"))
        if name:
            context.load_cert_chain(self.file(name, ".pem"), self.file(name, ".key"))
        return context

    def close(self):
        self.temp.cleanup()


@unittest.skipUnless(shutil.which("openssl"), "openssl is required for ephemeral loopback TLS tests")
class TLSTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture = TLSFixture()
        cls.addClassCleanup(cls.fixture.close)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="hefesto-server-test-")
        self.addCleanup(self.temp.cleanup)
        self.journal = hefesto.Journal(str(Path(self.temp.name) / "journal.sqlite"))
        self.addCleanup(self.journal.close)
        self.peers = {self.fixture.fingerprint("device"): DEVICE,
                      self.fixture.fingerprint("controller"): CONTROLLER}
        self.server = self.make_server()
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={"poll_interval": 0.01}, name="hefesto-test-listener")
        self.thread.start()
        self.addCleanup(self.stop_server)
        self.address = self.server.server_address

    def make_server(self, **changes):
        kwargs = {"certfile": self.fixture.file("server", ".pem"), "keyfile": self.fixture.file("server", ".key"),
                  "cafile": self.fixture.file("ca", ".pem"), **changes}
        return hefesto.create_server(("127.0.0.1", 0), self.journal, self.peers, **kwargs)

    def stop_server(self):
        self.server.shutdown()
        workers = list(self.server.__dict__.get("_threads", ()))
        watchdog = self.server.__dict__.get("_watchdog")
        self.server.server_close()
        self.thread.join(timeout=5)
        self.assertFalse(self.thread.is_alive())
        self.assertFalse(any(thread.is_alive() for thread in workers))
        self.assertTrue(watchdog is None or not watchdog.is_alive())

    def request(self, name, operation="status", fields=None):
        connection = http.client.HTTPSConnection(*self.address, context=self.fixture.context(name), timeout=5)
        try:
            connection.request("GET" if fields is None else "POST", PREFIX + operation,
                               body=None if fields is None else encoded(**fields),
                               headers={} if fields is None else {"Content-Type": "application/json"})
            response = connection.getresponse()
            return response.status, json.loads(response.read())
        finally:
            connection.close()

    def raw(self, request, name="device"):
        with socket.create_connection(self.address, timeout=5) as transport:
            with self.fixture.context(name).wrap_socket(transport, server_hostname="localhost") as connection:
                connection.sendall(request)
                response = http.client.HTTPResponse(connection)
                response.begin()
                return response.status, json.loads(response.read())

    def test_ca_and_explicit_fingerprint_roles_are_both_required(self):
        self.assertEqual(self.server._context.verify_mode, ssl.CERT_REQUIRED)
        self.assertGreaterEqual(self.server._context.minimum_version, ssl.TLSVersion.TLSv1_2)
        self.assertEqual(self.request("device")[0], 200)
        self.assertEqual(self.request("controller")[0], 200)
        for name in ("unlisted", None, "foreign"):
            with self.subTest(name=name):
                with self.assertRaises((ssl.SSLError, OSError, http.client.HTTPException)):
                    self.request(name)
        self.assertEqual(self.request("controller", "register", {"instanceId": "i"})[0], 403)
        self.assertEqual(self.request("device", "submit", submission())[0], 403)

    def test_real_tls_echo_round_trip_and_device_spoof_rejection(self):
        status, state = self.request("device", "register", {"instanceId": "tls-instance"})
        self.assertEqual(status, 200)
        self.assertEqual(state["deviceId"], DEVICE.device_id)
        self.assertEqual(self.request("device", "heartbeat", {"instanceId": "tls-instance", "deviceId": "other"})[0], 400)
        status, state = self.request("controller", "control", {"action": "resume", "expectedGeneration": state["generation"]})
        self.assertEqual(status, 200)
        status, ack = self.request("controller", "submit", submission(text="offline TLS echo"))
        self.assertEqual(status, 200)
        status, delivery = self.request("device", "next", {"instanceId": "tls-instance", "generation": state["generation"]})
        self.assertEqual(status, 200)
        fields = JournalTests.result_fields(delivery["invocation"])
        status, success = self.request("device", "result", fields)
        self.assertEqual((status, success["state"]), (200, "succeeded"))
        self.assertEqual(success["invocationId"], ack["invocationId"])
        self.assertEqual(self.request("device", "result", fields), (200, success))

    def test_large_read_result_body_passes_listener_http_layer(self):
        status, state = self.request("device", "register", {"instanceId": "tls-instance"})
        self.assertEqual(status, 200)
        status, state = self.request("controller", "control",
                                     {"action": "resume", "expectedGeneration": state["generation"]})
        self.assertEqual(status, 200)
        status, ack = self.request("controller", "submit",
                                   submission(toolId="hefesto.read_file",
                                              arguments={"path": "/Users/x/Documents/a.txt"}))
        self.assertEqual(status, 200)
        status, delivery = self.request("device", "next",
                                        {"instanceId": "tls-instance", "generation": state["generation"]})
        self.assertEqual(status, 200)
        fields = JournalTests.result_fields(delivery["invocation"], output={"text": "x" * 16384})
        status, success = self.request("device", "result", fields)
        self.assertEqual((status, success["state"]), (200, "succeeded"))
        self.assertEqual(success["invocationId"], ack["invocationId"])

    def test_plain_http_never_reaches_journal(self):
        with socket.create_connection(self.address, timeout=5) as connection:
            connection.sendall(b"GET /v1/hefesto/status HTTP/1.1\r\nHost: localhost\r\n\r\n")
            try:
                received = connection.recv(1024)
            except ConnectionResetError:
                received = b""
        self.assertNotIn(b"200", received)
        self.assertNotIn(b"deviceId", received)
        with contextlib.closing(sqlite3.connect(self.journal_path())) as db:
            self.assertEqual(db.execute("SELECT count(*) FROM devices").fetchone()[0], 0)

    def journal_path(self):
        return str(Path(self.temp.name) / "journal.sqlite")

    def test_unlisted_fingerprint_registry_copy_cannot_be_mutated_by_caller(self):
        self.peers[self.fixture.fingerprint("unlisted")] = DEVICE
        with self.assertRaises((ssl.SSLError, OSError, http.client.HTTPException)):
            self.request("unlisted")
        self.assertEqual(self.request("device")[0], 200)

    def test_unlisted_certificate_is_dropped_before_any_http_exchange(self):
        with socket.create_connection(self.address, timeout=5) as transport:
            start = time.monotonic()
            with self.fixture.context("unlisted").wrap_socket(transport, server_hostname="localhost") as connection:
                try:
                    received = connection.recv(1)
                except (ssl.SSLError, OSError):
                    received = b""
            self.assertEqual(received, b"")
            self.assertLess(time.monotonic() - start, hefesto.CONNECTION_TIMEOUT + 2)
        with contextlib.closing(sqlite3.connect(self.journal_path())) as db:
            self.assertEqual(db.execute("SELECT count(*) FROM devices").fetchone()[0], 0)
        self.assertEqual(self.request("device")[0], 200)

    def test_absolute_deadline_closes_a_slow_trickle(self):
        self.server.request_deadline = 0.6
        with socket.create_connection(self.address, timeout=5) as transport:
            with self.fixture.context("device").wrap_socket(transport, server_hostname="localhost") as connection:
                start = time.monotonic()
                connection.sendall(b"GET /v1/hefesto/status HTTP/1.1\r\n")
                closed = None
                while closed is None and time.monotonic() - start < hefesto.CONNECTION_TIMEOUT + 1:
                    time.sleep(0.1)
                    try:
                        connection.sendall(b"X")
                    except OSError:
                        closed = time.monotonic() - start
                        break
                    connection.settimeout(0.01)
                    try:
                        if connection.recv(1) == b"":
                            closed = time.monotonic() - start
                    except ssl.SSLWantReadError:
                        pass
                    except TimeoutError:
                        pass
                    except OSError:
                        closed = time.monotonic() - start
                    finally:
                        connection.settimeout(5)
                self.assertIsNotNone(closed)
                self.assertLess(closed, hefesto.CONNECTION_TIMEOUT)
        self.assertEqual(self.request("device")[0], 200)

    def test_stalled_handshake_does_not_block_accept_and_times_out(self):
        with socket.create_connection(self.address, timeout=5) as stalled:
            start = time.monotonic()
            self.assertEqual(self.request("device")[0], 200)
            self.assertLess(time.monotonic() - start, hefesto.CONNECTION_TIMEOUT)
            self.assertEqual(stalled.recv(1), b"")
            self.assertLess(time.monotonic() - start, hefesto.CONNECTION_TIMEOUT + 2)

    def test_shutdown_closes_stalled_handshake_and_joins_workers(self):
        with socket.create_connection(self.address, timeout=5) as stalled:
            self.assertEqual(self.request("device")[0], 200)
            start = time.monotonic()
            self.stop_server()
            self.assertLess(time.monotonic() - start, 2)
            self.assertEqual(stalled.recv(1), b"")
        self.assertEqual(self.journal.dispatch(DEVICE, "GET", PREFIX + "status")[0], 200)

    def test_partial_body_timeout_returns_fixed_error(self):
        with socket.create_connection(self.address, timeout=5) as transport:
            with self.fixture.context("device").wrap_socket(transport, server_hostname="localhost") as connection:
                connection.sendall(b"POST /v1/hefesto/register HTTP/1.1\r\nHost: localhost\r\n"
                                   b"Content-Type: application/json\r\nContent-Length: 100\r\n\r\n{")
                response = http.client.HTTPResponse(connection)
                response.begin()
                self.assertEqual(response.status, 408)
                self.assertEqual(json.loads(response.read()), {"protocolVersion": 3, "error": "request_timeout"})

    def test_smuggling_framing_and_duplicate_headers_are_rejected(self):
        cases = (b"Content-Length: 2\r\nContent-Length: 2\r\n", b"Content-Length: 2\r\ncontent-length: 3\r\n",
                 b"Transfer-Encoding: chunked\r\nContent-Length: 2\r\n", b"Transfer-Encoding: identity\r\n",
                 b"Trailer: X-Other\r\n", b"Expect: 100-continue\r\n", b"Upgrade: h2c\r\n",
                 b"Content-Length: -1\r\n", b"Content-Length: +2\r\n", b"Content-Length: 2, 2\r\n",
                 b"Content-Length: 2\r\nHost: other\r\n", b"Content-Length: 2\r\nX-Fold: a\r\n b\r\n")
        for headers in cases:
            with self.subTest(headers=headers):
                status, error = self.raw(b"POST /v1/hefesto/register HTTP/1.1\r\nHost: localhost\r\n"
                                         b"Content-Type: application/json\r\n" + headers + b"\r\n{}")
                self.assertEqual(status, 400)
                self.assertEqual(error, {"protocolVersion": 3, "error": "invalid_http_framing"})

    def test_body_limit_content_type_length_paths_and_json_over_tls(self):
        base = b"POST /v1/hefesto/register HTTP/1.1\r\nHost: localhost\r\n"
        oversized = f"Content-Length: {hefesto.MAX_BODY_BYTES + 1}\r\n".encode()
        for headers, body, expected in ((oversized, b"", 413),
                                        (b"Content-Type: application/json\r\n", b"", 411),
                                        (b"Content-Length: 2\r\nContent-Type: text/plain\r\n", b"{}", 415),
                                        (b"Content-Length: 2\r\nContent-Type: application/json; charset=latin1\r\n", b"{}", 415)):
            self.assertEqual(self.raw(base + headers + b"\r\n" + body)[0], expected)
        for body in (b'{"protocolVersion":3,"instanceId":"i","instanceId":"other"}', b"\xff"):
            status, error = self.raw(base + b"Content-Type: application/json\r\nContent-Length: "
                                     + str(len(body)).encode() + b"\r\n\r\n" + body)
            self.assertEqual(status, 400)
            self.assertEqual(error["error"], "invalid_json")
        self.assertEqual(self.raw(b"GET /v1/hefesto/status HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}")[0], 400)
        self.assertEqual(self.raw(b"GET //v1/hefesto/status HTTP/1.1\r\nHost: localhost\r\n\r\n")[0], 400)
        self.assertEqual(self.raw(b"GET /v1/hefesto/status?deviceId=other HTTP/1.1\r\nHost: localhost\r\n\r\n")[0], 404)
        self.assertEqual(self.raw(b"GET /v1/hefesto/status HTTP/1.1\r\n\r\n")[0], 400)

    def test_no_raw_access_error_logs_or_exception_details(self):
        output = io.StringIO()
        with contextlib.redirect_stderr(output), contextlib.redirect_stdout(output):
            status, error = self.raw(b"POST /sensitive-secret HTTP/1.1\r\nHost: localhost\r\n"
                                     b"Content-Type: application/json\r\nContent-Length: 2\r\n\r\n{}")
            self.assertEqual((status, error["error"]), (404, "not_found"))
            with mock.patch.object(self.journal, "_operate", side_effect=RuntimeError("sensitive-secret")):
                status, error = self.request("device")
            self.assertEqual((status, error["error"]), (503, "journal_unavailable"))
        self.assertEqual(output.getvalue(), "")
        self.assertNotIn("sensitive-secret", json.dumps(error))

    def test_factory_errors_are_sanitized_and_does_not_start_threads(self):
        for changes in ({"certfile": "/missing/private/cert.pem"}, {"keyfile": "/missing/private/key.pem"},
                        {"cafile": "/missing/private/ca.pem"}, {"cafile": None}):
            with self.assertRaisesRegex(ValueError, "^tls_configuration_error$"):
                self.make_server(**changes)
        previous_threads = {thread.ident for thread in threading.enumerate()}
        server = self.make_server()
        try:
            self.assertEqual({thread.ident for thread in threading.enumerate()}, previous_threads)
        finally:
            server.server_close()
        self.assertEqual(self.journal.dispatch(DEVICE, "GET", PREFIX + "status")[0], 200)


if __name__ == "__main__":
    unittest.main()
