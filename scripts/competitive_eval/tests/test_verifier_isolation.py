from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from scripts.competitive_eval.adapters.mock import MockAdapter
from scripts.competitive_eval.manifest import load_task_manifest
from scripts.competitive_eval.run_one import TrialConfig, run_trial

from .helpers import write_fixture


class VerifierIsolationTests(unittest.TestCase):
    def test_each_trial_gets_fresh_workspace_home_and_temp_directories(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest_path = write_fixture(root, "pass")
            manifest = load_task_manifest(manifest_path)
            source = Path(manifest["fixture"]["source"])
            source_snapshot = sorted(path.relative_to(source) for path in source.rglob("*"))
            run_root = root / "results"
            first = run_trial(
                manifest,
                manifest_path=manifest_path,
                adapter=MockAdapter(),
                config=TrialConfig("campaign", 1, run_root / "first", output_limit_bytes=8192),
            )
            second = run_trial(
                manifest,
                manifest_path=manifest_path,
                adapter=MockAdapter(),
                config=TrialConfig("campaign", 2, run_root / "second", output_limit_bytes=8192),
            )
            self.assertEqual(first["status"], "pass")
            self.assertEqual(second["status"], "pass")
            first_env = json.loads((run_root / "first" / "environment.json").read_text(encoding="utf-8"))
            second_env = json.loads((run_root / "second" / "environment.json").read_text(encoding="utf-8"))
            self.assertNotEqual(first_env["HOME"], second_env["HOME"])
            self.assertNotEqual(first_env["TMPDIR"], second_env["TMPDIR"])
            self.assertNotEqual(first["workspace"], second["workspace"])
            self.assertFalse((source / "solution.txt").exists())
            self.assertEqual(source_snapshot, sorted(path.relative_to(source) for path in source.rglob("*")))

    def test_fixture_tree_limits_are_enforced(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest_path = write_fixture(root, "pass")
            manifest = load_task_manifest(manifest_path)
            with self.assertRaisesRegex(ValueError, "fixture file limit"):
                run_trial(
                    manifest,
                    manifest_path=manifest_path,
                    adapter=MockAdapter(),
                    config=TrialConfig("campaign", 1, root / "result", max_fixture_files=1),
                )

    def test_fixture_symlinks_are_rejected_instead_of_pointing_back_to_source(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest_path = write_fixture(root, "pass")
            manifest = load_task_manifest(manifest_path)
            source = Path(manifest["fixture"]["source"])
            (source / "link").symlink_to(source / "prompt.md")
            with self.assertRaisesRegex(ValueError, "symlinks are unsupported"):
                run_trial(
                    manifest,
                    manifest_path=manifest_path,
                    adapter=MockAdapter(),
                    config=TrialConfig("campaign", 1, root / "result"),
                )


if __name__ == "__main__":
    unittest.main()
