from __future__ import annotations

import subprocess
from pathlib import Path


root = Path.cwd()
lib = (root / "src/lib.rs").read_text(encoding="utf-8")
math = (root / "src/math.rs").read_text(encoding="utf-8")
if "calculate" in lib or "calculate" in math:
    raise SystemExit("old symbol remains")
if "math::compute(20, 22)" not in lib:
    raise SystemExit("call site was not renamed")
if "pub fn compute(" not in math:
    raise SystemExit("definition was not renamed")
raise SystemExit(subprocess.run(["cargo", "check", "--quiet"], cwd=root).returncode)
