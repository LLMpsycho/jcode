from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
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


def supports_step_in_targets(adapter: Path) -> bool:
    request = {
        "seq": 1,
        "type": "request",
        "command": "initialize",
        "arguments": {
            "clientID": "jcode-eval",
            "adapterID": "lldb",
            "pathFormat": "path",
            "linesStartAt1": True,
            "columnsStartAt1": True,
            "supportsRunInTerminalRequest": False,
        },
    }
    payload = json.dumps(request, separators=(",", ":")).encode()
    frame = f"Content-Length: {len(payload)}\r\n\r\n".encode() + payload
    try:
        probe = subprocess.run(
            [adapter],
            input=frame,
            check=False,
            capture_output=True,
            timeout=5,
        )
        if len(probe.stdout) > 1024 * 1024:
            return False
        header, body = probe.stdout.split(b"\r\n\r\n", 1)
        content_length = next(
            int(line.split(b":", 1)[1])
            for line in header.split(b"\r\n")
            if line.lower().startswith(b"content-length:")
        )
        if content_length > 1024 * 1024 or len(body) < content_length:
            return False
        response = json.loads(body[:content_length])
    except (OSError, ValueError, StopIteration, subprocess.TimeoutExpired):
        return False
    return response.get("body", {}).get("supportsStepInTargetsRequest") is True


adapter = find_lldb_dap()
if not supports_step_in_targets(adapter):
    print(
        "lldb-dap does not advertise supportsStepInTargetsRequest",
        file=sys.stderr,
    )
    raise SystemExit(78)
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
