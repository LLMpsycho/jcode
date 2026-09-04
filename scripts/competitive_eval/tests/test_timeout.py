from __future__ import annotations

import os
import sys
import tempfile
import time
import unittest
from pathlib import Path

from scripts.competitive_eval.process_metrics import run_process


def process_exists(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


@unittest.skipUnless(os.name == "posix", "process-group assertions require POSIX")
class ProcessTimeoutTests(unittest.TestCase):
    def test_timeout_kills_parent_and_spawned_child_process_group(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            pid_file = Path(tmp) / "child.pid"
            script = (
                "import pathlib, subprocess, sys, time; "
                "p=subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)']); "
                "pathlib.Path(sys.argv[1]).write_text(str(p.pid)); "
                "time.sleep(60)"
            )
            outcome = run_process(
                [sys.executable, "-c", script, str(pid_file)],
                cwd=Path(tmp),
                env=os.environ.copy(),
                timeout_seconds=0.25,
                terminate_grace_seconds=0.05,
            )
            self.assertTrue(outcome.timed_out)
            child_pid = int(pid_file.read_text(encoding="utf-8"))
            deadline = time.monotonic() + 2
            while process_exists(child_pid) and time.monotonic() < deadline:
                time.sleep(0.02)
            self.assertFalse(process_exists(child_pid), f"child {child_pid} leaked after timeout")

    def test_successful_parent_still_cleans_background_descendant(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            pid_file = Path(tmp) / "child.pid"
            script = (
                "import pathlib, subprocess, sys; "
                "p=subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)']); "
                "pathlib.Path(sys.argv[1]).write_text(str(p.pid))"
            )
            outcome = run_process(
                [sys.executable, "-c", script, str(pid_file)],
                cwd=Path(tmp),
                env=os.environ.copy(),
                timeout_seconds=2,
                terminate_grace_seconds=0.05,
            )
            self.assertEqual(outcome.returncode, 0)
            child_pid = int(pid_file.read_text(encoding="utf-8"))
            deadline = time.monotonic() + 2
            while process_exists(child_pid) and time.monotonic() < deadline:
                time.sleep(0.02)
            self.assertFalse(process_exists(child_pid), f"background child {child_pid} leaked")

    def test_stdout_and_stderr_are_capped_without_deadlock(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            script = "import sys; sys.stdout.write('o'*200000); sys.stderr.write('e'*200000)"
            outcome = run_process(
                [sys.executable, "-c", script],
                cwd=Path(tmp),
                env=os.environ.copy(),
                timeout_seconds=2,
                output_limit_bytes=4096,
            )
        self.assertEqual(outcome.returncode, 0)
        self.assertEqual(len(outcome.stdout), 4096)
        self.assertEqual(len(outcome.stderr), 4096)
        self.assertTrue(outcome.stdout_truncated)
        self.assertTrue(outcome.stderr_truncated)


if __name__ == "__main__":
    unittest.main()
