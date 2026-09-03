from __future__ import annotations

import stat
import tempfile
import unittest
from pathlib import Path

from scripts.competitive_eval.adapters.base import RunSpec
from scripts.competitive_eval.adapters.jcode import JcodeAdapter
from scripts.competitive_eval.adapters.omp import OmpAdapter


class AdapterTests(unittest.TestCase):
    def make_binary(self, root: Path, name: str) -> Path:
        path = root / name
        path.write_text("#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'tool v1.2.3 (abc123)'; fi\n", encoding="utf-8")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
        return path

    def run_spec(self, root: Path) -> RunSpec:
        workspace = root / "workspace"
        home = root / "home"
        run_dir = root / "run"
        for path in (workspace, home, run_dir):
            path.mkdir()
        prompt_file = workspace / "prompt.md"
        prompt_file.write_text("do the task", encoding="utf-8")
        return RunSpec(
            campaign_id="campaign",
            task_id="task",
            attempt=1,
            workspace=workspace,
            home=home,
            temp_dir=root / "tmp",
            run_dir=run_dir,
            prompt_file=prompt_file,
            prompt="do the task",
            timeout_seconds=30,
            provider="provider",
            model="model",
            reasoning_effort="high",
            service_tier=None,
            required_capabilities=(),
            tags=(),
        )

    def test_jcode_adapter_requires_explicit_binary_and_isolates_socket(self) -> None:
        self.assertFalse(JcodeAdapter(None).probe().supported)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary = self.make_binary(root, "jcode")
            adapter = JcodeAdapter(binary)
            spec = self.run_spec(root)
            command = adapter.build_command(spec)
            environment = adapter.environment(spec)
            version = adapter.version()
            self.assertEqual(command[0], str(binary.resolve()))
            self.assertIn("--socket", command)
            self.assertTrue(environment["JCODE_SOCKET"].startswith(str(spec.run_dir)))
            self.assertEqual(len(version.binary_sha256), 64)
            self.assertIn("v1.2.3", version.version)

    def test_omp_adapter_requires_explicit_binary_and_fresh_config(self) -> None:
        self.assertFalse(OmpAdapter(None).probe().supported)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary = self.make_binary(root, "omp")
            adapter = OmpAdapter(binary)
            spec = self.run_spec(root)
            command = adapter.build_command(spec)
            environment = adapter.environment(spec)
            self.assertEqual(command[0], str(binary.resolve()))
            self.assertIn(spec.prompt, command)
            self.assertEqual(environment["OMP_HOME"], str(spec.home))
            self.assertTrue(environment["XDG_CONFIG_HOME"].startswith(str(spec.home)))


if __name__ == "__main__":
    unittest.main()
