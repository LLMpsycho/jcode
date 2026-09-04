from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from scripts.competitive_eval.adapters.mock import MockAdapter
from scripts.competitive_eval.manifest import load_task_manifest
from scripts.competitive_eval.run_one import TrialConfig, run_trial

from scripts.competitive_eval.tests.helpers import write_fixture


class MockOutcomeTests(unittest.TestCase):
    def test_setup_can_report_an_unsupported_runtime_capability(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest_path = write_fixture(root, "setup-unsupported")
            manifest = load_task_manifest(manifest_path)
            fixture = manifest_path.parent / manifest["fixture"]["source"]
            (fixture / "setup.py").write_text("raise SystemExit(78)\n", encoding="utf-8")
            manifest["setup"] = {"command": "python3 setup.py"}
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            result = run_trial(
                load_task_manifest(manifest_path),
                manifest_path=manifest_path,
                adapter=MockAdapter(),
                config=TrialConfig("campaign", 1, root / "result"),
            )
            self.assertEqual(result["status"], "unsupported")
            self.assertEqual(result["failure_class"], "unsupported_capability")
            self.assertIn("runtime capability", result["unsupported_reason"])

    def test_pass_fail_timeout_crash_and_unsupported_are_recorded(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cases = {
                "pass": ("pass", None),
                "fail": ("fail", "verifier"),
                "timeout": ("timeout", "agent_timeout"),
                "crash": ("crash", "agent_exit"),
                "unsupported": ("unsupported", "unsupported_capability"),
            }
            for behavior, (expected_status, expected_failure) in cases.items():
                with self.subTest(behavior=behavior):
                    required = ["nonexistent-capability"] if behavior == "unsupported" else []
                    manifest_path = write_fixture(root, behavior, required_capabilities=required)
                    result_dir = root / "results" / behavior
                    result = run_trial(
                        load_task_manifest(manifest_path),
                        manifest_path=manifest_path,
                        adapter=MockAdapter(),
                        config=TrialConfig("campaign", 1, result_dir, output_limit_bytes=2048),
                    )
                    self.assertEqual(result["status"], expected_status)
                    self.assertEqual(result["failure_class"], expected_failure)
                    stored = json.loads((result_dir / "result.json").read_text(encoding="utf-8"))
                    self.assertEqual(stored["status"], expected_status)

    def test_large_mock_output_is_capped_and_marked(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest_path = write_fixture(root, "large-output")
            result_dir = root / "result"
            result = run_trial(
                load_task_manifest(manifest_path),
                manifest_path=manifest_path,
                adapter=MockAdapter(),
                config=TrialConfig("campaign", 1, result_dir, output_limit_bytes=1024),
            )
            self.assertEqual(result["status"], "pass")
            self.assertTrue(result["output_truncated"])
            self.assertLessEqual((result_dir / "stdout.log").stat().st_size, 1024)
            self.assertLessEqual((result_dir / "stderr.log").stat().st_size, 1024)


if __name__ == "__main__":
    unittest.main()
