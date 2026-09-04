from __future__ import annotations

import subprocess
from pathlib import Path


root = Path.cwd()
source = (root / "src/main.rs").read_text(encoding="utf-8")
required = (
    "fn read_seed() -> i32 {\n    6\n}",
    "fn scale(seed: i32) -> i32 {\n    seed * 7\n}",
    "fn finalize(value: i32) -> i32 {\n    value\n}",
    "finalize(scale(read_seed()))",
)
for fragment in required:
    if fragment not in source:
        raise SystemExit(f"required structure was removed: {fragment}")
for forbidden in ("42", "eprintln!", "dbg!"):
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
    raise SystemExit(f"program fails: {completed.stderr}")
if completed.stdout.strip() != "42":
    raise SystemExit(f"expected 42, got {completed.stdout!r}")
