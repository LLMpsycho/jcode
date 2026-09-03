from __future__ import annotations

import subprocess
import sys
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]


class DirectCliTests(unittest.TestCase):
    def test_cli_files_support_direct_help_invocation(self) -> None:
        for name in ("run_campaign.py", "run_one.py", "compare.py", "summarize.py", "redact.py"):
            with self.subTest(name=name):
                completed = subprocess.run(
                    [sys.executable, str(REPO_ROOT / "scripts" / "competitive_eval" / name), "--help"],
                    cwd=REPO_ROOT,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                    timeout=10,
                    check=False,
                )
                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assertIn("usage:", completed.stdout.lower())


if __name__ == "__main__":
    unittest.main()
