#!/usr/bin/env python3
"""Security and provenance checks for the advisor acceptance report."""

from contextlib import redirect_stderr, redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import test_advisor_acceptance as acceptance


class AcceptanceReportTests(unittest.TestCase):
    def test_report_redacts_credentials_before_stdout_and_atomic_private_write(self):
        key = "unrecognized-provider-credential"
        token = "sk-proj-" + "x" * 32
        report = {"results": [{"advisor_verdict": {"inspect": (
            f"{key} {token} {acceptance.SYNTHETIC_SECRET} Evidence: tests were not run."
        )}}]}
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ, {"OPENAI_API_KEY": key}):
            path = Path(tmp) / "report.json"
            path.write_text("old report")
            path.chmod(0o644)
            stdout = io.StringIO()
            with redirect_stdout(stdout):
                acceptance.emit_report(report, path)
            self.assertEqual(stdout.getvalue(), path.read_text())
            for secret in (key, token, acceptance.SYNTHETIC_SECRET):
                self.assertNotIn(secret, stdout.getvalue())
            self.assertIn("Evidence: tests were not run.", stdout.getvalue())
            if os.name == "posix":
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(list(Path(tmp).iterdir()), [path])

    def test_oversized_report_cannot_replace_prior_report_or_reach_stdout(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "report.json"
            path.write_text("prior report")
            stdout = io.StringIO()
            with redirect_stdout(stdout), self.assertRaises(ValueError):
                acceptance.emit_report({"inspect": "x" * acceptance.MAX_REPORT_BYTES}, path)
            self.assertEqual(stdout.getvalue(), "")
            self.assertEqual(path.read_text(), "prior report")

    def test_live_requires_credential_before_binary_resolution_or_fixture_creation(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(acceptance, "Fixture") as fixture:
            with redirect_stderr(io.StringIO()) as stderr, self.assertRaises(SystemExit) as error:
                acceptance.main(["--live", "--binary", "/missing-advisor-binary"])
            self.assertEqual(error.exception.code, 2)
            self.assertIn("requires an existing OPENAI_API_KEY", stderr.getvalue())
            fixture.assert_not_called()

    def test_live_report_records_requested_model_binary_and_each_verdict_without_fixture(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "jcode"
            binary.write_bytes(b"test binary identity")
            report = Path(tmp) / "report.json"
            def review(_binary, mode, fixture, model):
                self.assertIsNone(fixture)
                self.assertEqual(model, "requested-review-model")
                return {"mode": mode, "status": "passed", "advisor_verdict": {"inspect": mode}}
            with patch.dict(os.environ, {"OPENAI_API_KEY": "test-key"}), \
                    patch.object(acceptance, "Fixture") as fixture, \
                    patch.object(acceptance, "exercise", side_effect=review), \
                    redirect_stdout(io.StringIO()):
                acceptance.main(["--live", "--binary", str(binary), "--report", str(report),
                                 "--model", "requested-review-model"])
            data = json.loads(report.read_text())
            self.assertEqual(data["provider"], "live-openai")
            self.assertEqual(data["requested_model"], "requested-review-model")
            self.assertEqual(data["binary_sha256"], hashlib.sha256(binary.read_bytes()).hexdigest())
            self.assertEqual([item["mode"] for item in data["results"]], list(acceptance.MODES))
            self.assertTrue(all(item["advisor_verdict"]["inspect"] for item in data["results"]))
            fixture.assert_not_called()

    def test_failed_live_review_does_not_publish_a_success_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "jcode"
            binary.write_bytes(b"test binary identity")
            report = Path(tmp) / "report.json"
            with patch.dict(os.environ, {"OPENAI_API_KEY": "test-key"}), \
                    patch.object(acceptance, "exercise", side_effect=TimeoutError("review timed out")), \
                    redirect_stdout(io.StringIO()) as stdout, self.assertRaises(TimeoutError):
                acceptance.main(["--live", "--binary", str(binary), "--report", str(report)])
            self.assertFalse(report.exists())
            self.assertEqual(stdout.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
