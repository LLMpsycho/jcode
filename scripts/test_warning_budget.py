#!/usr/bin/env python3
"""Hermetic regression coverage for the compiler warning gate."""

from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


CHECKER = Path(__file__).with_name("check_warning_budget.sh")


class WarningBudgetTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="jcode-warning-budget-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        self.checker = scripts / CHECKER.name
        shutil.copyfile(CHECKER, self.checker)
        self.baseline = scripts / "warning_budget.txt"
        self.baseline.write_text("0\n", encoding="utf-8")
        binary_dir = self.root / "bin"
        binary_dir.mkdir()
        fake_cargo = binary_dir / "cargo"
        fake_cargo.write_text(
            "#!/bin/sh\n"
            '[ "$*" = "check -q" ] || exit 98\n'
            '[ "$CARGO_TERM_COLOR" = "never" ] || exit 99\n'
            'printf "%s" "$JCODE_TEST_CARGO_OUTPUT" >&2\n'
            'exit "$JCODE_TEST_CARGO_STATUS"\n',
            encoding="utf-8",
        )
        fake_cargo.chmod(0o700)
        self.environment = {
            **os.environ,
            "PATH": str(binary_dir) + os.pathsep + os.environ.get("PATH", ""),
            "TMPDIR": str(self.root),
        }

    def run_check(
        self, status: int = 0, output: str = "", *arguments: str
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(self.checker), *arguments],
            env={
                **self.environment,
                "JCODE_TEST_CARGO_STATUS": str(status),
                "JCODE_TEST_CARGO_OUTPUT": output,
            },
            text=True,
            capture_output=True,
            check=False,
            timeout=10,
        )

    def test_failed_compiler_with_zero_warnings_fails(self) -> None:
        result = self.run_check(101, "error: fixture compilation failed\n")
        self.assertEqual(result.returncode, 101)
        self.assertIn("fixture compilation failed", result.stderr)
        self.assertNotIn("Warning budget OK", result.stdout)

    def test_failed_compiler_cannot_update_the_baseline(self) -> None:
        result = self.run_check(101, "warning: fixture warning\n", "--update")
        self.assertEqual(result.returncode, 101)
        self.assertEqual(self.baseline.read_text(encoding="utf-8"), "0\n")

    def test_success_without_warnings_passes(self) -> None:
        result = self.run_check(0, "Checking fixture v0.1.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Warning budget OK: current=0 baseline=0", result.stdout)

    def test_success_with_one_warning_exceeds_zero_budget(self) -> None:
        result = self.run_check(0, "warning: fixture warning\n")
        self.assertEqual(result.returncode, 1)
        self.assertIn("current=1 baseline=0", result.stderr)

    def test_success_matching_existing_budget_passes(self) -> None:
        self.baseline.write_text("1\n", encoding="utf-8")
        result = self.run_check(0, "warning: fixture warning\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Warning budget OK: current=1 baseline=1", result.stdout)


if __name__ == "__main__":
    unittest.main()
