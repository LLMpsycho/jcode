from __future__ import annotations

import subprocess
from pathlib import Path


root = Path.cwd()
source = (root / "src/main.rs").read_text(encoding="utf-8")
required = (
    "fn select_label<'a>(requested_slot: usize, labels: &'a [&'a str]) -> &'a str",
    "let storage_slot = requested_slot;",
    "labels[storage_slot]",
    'let labels = ["alpha", "beta", "gamma"];',
    'select_label(2, &labels)',
)
for fragment in required:
    if fragment not in source:
        raise SystemExit(f"required repair structure is missing: {fragment}")
for forbidden in ("println!(\"gamma\")", "eprintln!", "dbg!"):
    if forbidden in source:
        raise SystemExit(f"forbidden shortcut remains: {forbidden}")
completed = subprocess.run(
    ["cargo", "run", "--quiet"],
    cwd=root,
    check=False,
    capture_output=True,
    text=True,
)
if completed.returncode != 0:
    raise SystemExit(f"program still fails: {completed.stderr}")
if completed.stdout.strip() != "gamma":
    raise SystemExit(f"expected gamma, got {completed.stdout!r}")
