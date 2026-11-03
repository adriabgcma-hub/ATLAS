import io
import os
import tempfile
import types
import unittest
from contextlib import redirect_stderr

import atlas_relay


class LogCapTests(unittest.TestCase):
    def test_el_log_se_corta_a_las_300_lineas_mas_recientes(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "relay.log")
            brain = types.SimpleNamespace(log_path=path)
            with redirect_stderr(io.StringIO()):
                for number in range(305):
                    atlas_relay.dlog(brain, "TEST", f"linea {number}")
            with open(path, encoding="utf-8") as fh:
                lines = fh.readlines()
        self.assertEqual(len(lines), atlas_relay.MAX_LOG_LINES)
        self.assertIn("linea 5\n", lines[0])
        self.assertIn("linea 304\n", lines[-1])

    def test_el_log_corto_no_se_reescribe(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "relay.log")
            brain = types.SimpleNamespace(log_path=path)
            with redirect_stderr(io.StringIO()):
                for number in range(3):
                    atlas_relay.dlog(brain, "TEST", f"linea {number}")
            with open(path, encoding="utf-8") as fh:
                lines = fh.readlines()
        self.assertEqual(len(lines), 3)


if __name__ == "__main__":
    unittest.main()
