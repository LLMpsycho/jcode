from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path


def find_lldb_dap() -> Path:
    configured = os.environ.get("JCODE_EVAL_LLDB_DAP")
    candidates: list[str] = []
    if configured:
        candidates.append(configured)
    discovered = shutil.which("lldb-dap")
    if discovered:
        candidates.append(discovered)
    xcrun = shutil.which("xcrun")
    if xcrun:
        probe = subprocess.run(
            [xcrun, "--find", "lldb-dap"],
            check=False,
            capture_output=True,
            text=True,
        )
        if probe.returncode == 0:
            candidates.append(probe.stdout.strip())
    for candidate in candidates:
        path = Path(candidate).expanduser().resolve()
        if path.is_file() and os.access(path, os.X_OK):
            return path
    raise SystemExit("lldb-dap is required; set JCODE_EVAL_LLDB_DAP to its absolute path")


adapter = find_lldb_dap()
jcode_home = os.environ.get("JCODE_HOME")
if jcode_home:
    home = Path(jcode_home)
    home.mkdir(parents=True, exist_ok=True)
    (home / "config.toml").write_text(
        "[dap]\n"
        "enabled = true\n"
        "allow_evaluate = false\n\n"
        "[dap.adapters.lldb-dap]\n"
        "kind = \"lldb-dap\"\n"
        f"command = {json.dumps(str(adapter))}\n",
        encoding="utf-8",
    )
raise SystemExit(subprocess.run(["cargo", "build", "--quiet"], check=False).returncode)
