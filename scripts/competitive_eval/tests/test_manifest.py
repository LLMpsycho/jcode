from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from scripts.competitive_eval.manifest import (
    BaselineLockError,
    ManifestError,
    load_task_manifest,
    validate_baseline_lock,
)
from scripts.competitive_eval.run_one import atomic_write_json

from scripts.competitive_eval.tests.helpers import write_fixture


REPO_ROOT = Path(__file__).resolve().parents[3]


class ManifestValidationTests(unittest.TestCase):
    def test_valid_task_matches_locked_schema(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            manifest_path = write_fixture(Path(tmp), "pass")
            manifest = load_task_manifest(manifest_path)
        self.assertEqual(manifest["id"], "mock-pass")

    def test_missing_required_field_is_rejected_with_path(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            manifest_path = write_fixture(Path(tmp), "pass")
            data = json.loads(manifest_path.read_text(encoding="utf-8"))
            del data["verifier"]
            manifest_path.write_text(json.dumps(data), encoding="utf-8")
            with self.assertRaisesRegex(ManifestError, r"\$.*verifier"):
                load_task_manifest(manifest_path)

    def test_unknown_task_field_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            manifest_path = write_fixture(Path(tmp), "pass")
            data = json.loads(manifest_path.read_text(encoding="utf-8"))
            data["surprise"] = True
            manifest_path.write_text(json.dumps(data), encoding="utf-8")
            with self.assertRaisesRegex(ManifestError, "additional property"):
                load_task_manifest(manifest_path)

    def test_invalid_task_identifier_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            manifest_path = write_fixture(Path(tmp), "pass")
            data = json.loads(manifest_path.read_text(encoding="utf-8"))
            data["id"] = "Bad ID"
            manifest_path.write_text(json.dumps(data), encoding="utf-8")
            with self.assertRaisesRegex(ManifestError, "pattern"):
                load_task_manifest(manifest_path)

    def test_repository_dap_fixtures_are_valid_and_require_debugger(self) -> None:
        paths = sorted(
            (REPO_ROOT / "scripts/competitive_eval/fixtures/dap").glob("*/task.json")
        )
        self.assertEqual(len(paths), 2)
        identifiers = set()
        for path in paths:
            manifest = load_task_manifest(path)
            identifiers.add(manifest["id"])
            self.assertIn("debugger", manifest["agent"]["required_capabilities"])
            self.assertEqual(manifest["setup"]["command"], "python3 setup.py")
            self.assertTrue((path.parent / manifest["prompt_file"]).is_file())
            self.assertTrue((path.parent / "setup.py").is_file())
            self.assertTrue((path.parent / "verify.py").is_file())
        self.assertEqual(len(identifiers), len(paths))


class BaselineLockTests(unittest.TestCase):
    def test_repository_baseline_lock_is_valid(self) -> None:
        checked = validate_baseline_lock(REPO_ROOT)
        self.assertEqual(len(checked), 6)

    def test_changed_locked_file_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            locked = root / "locked.txt"
            locked.write_text("original", encoding="utf-8")
            digest = hashlib.sha256(locked.read_bytes()).hexdigest()
            lock_dir = root / "competitive-eval" / "baselines"
            lock_dir.mkdir(parents=True)
            (lock_dir / "baseline.lock.json").write_text(
                json.dumps({"schema_version": 1, "locked_files": [{"path": "locked.txt", "sha256": digest}]}),
                encoding="utf-8",
            )
            locked.write_text("changed", encoding="utf-8")
            with self.assertRaisesRegex(BaselineLockError, "hash mismatch"):
                validate_baseline_lock(root)

    def test_lock_cannot_escape_repository(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            lock_dir = root / "competitive-eval" / "baselines"
            lock_dir.mkdir(parents=True)
            (lock_dir / "baseline.lock.json").write_text(
                json.dumps({"schema_version": 1, "locked_files": [{"path": "../outside", "sha256": "0" * 64}]}),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(BaselineLockError, "escapes"):
                validate_baseline_lock(root)


class AtomicResultTests(unittest.TestCase):
    def test_atomic_writer_always_leaves_valid_json(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "nested" / "result.json"
            for index in range(25):
                atomic_write_json(path, {"index": index, "payload": "x" * index})
                self.assertEqual(json.loads(path.read_text(encoding="utf-8"))["index"], index)
                self.assertEqual(list(path.parent.glob(".result.json.*.tmp")), [])


if __name__ == "__main__":
    unittest.main()
