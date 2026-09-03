from __future__ import annotations

import importlib.util
import subprocess
from pathlib import Path


root = Path.cwd()
source = root / "src/app.py"
typecheck = subprocess.run(
    ["pyright", "--outputjson", str(source)],
    cwd=root,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
)
if typecheck.returncode != 0:
    raise SystemExit("pyright still reports errors")
spec = importlib.util.spec_from_file_location("fixture_app", source)
if spec is None or spec.loader is None:
    raise SystemExit("cannot import repaired module")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
if module.greet("Ada") != "Ada!":
    raise SystemExit("greet behavior is incorrect")
