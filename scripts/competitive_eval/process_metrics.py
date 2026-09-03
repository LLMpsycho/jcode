from __future__ import annotations

import os
import json
import re
import signal
import subprocess
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import BinaryIO

from .adapters.base import AgentMetrics, ArtifactSet


@dataclass(frozen=True)
class ProcessOutcome:
    returncode: int | None
    duration_ms: int
    stdout: bytes
    stderr: bytes
    stdout_truncated: bool
    stderr_truncated: bool
    timed_out: bool
    peak_rss_bytes: int = 0


class _CappedReader:
    def __init__(self, stream: BinaryIO, limit: int) -> None:
        self.stream = stream
        self.limit = max(0, limit)
        self.data = bytearray()
        self.truncated = False

    def run(self) -> None:
        try:
            while True:
                chunk = self.stream.read(64 * 1024)
                if not chunk:
                    return
                remaining = self.limit - len(self.data)
                if remaining > 0:
                    self.data.extend(chunk[:remaining])
                if len(chunk) > remaining:
                    self.truncated = True
        finally:
            self.stream.close()


def _group_exists(pgid: int) -> bool:
    if os.name != "posix":
        return False
    try:
        os.killpg(pgid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def terminate_process_group(
    process: subprocess.Popen[bytes],
    pgid: int | None = None,
    grace_seconds: float = 0.2,
) -> None:
    if os.name == "posix":
        group = pgid if pgid is not None else process.pid
        if not _group_exists(group):
            return
        try:
            os.killpg(group, signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            return
        if process.poll() is None:
            try:
                process.wait(timeout=max(0.0, grace_seconds))
            except subprocess.TimeoutExpired:
                pass
        if _group_exists(group):
            try:
                os.killpg(group, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
        return
    if os.name == "nt":
        subprocess.run(
            ["taskkill", "/PID", str(process.pid), "/T", "/F"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    elif process.poll() is None:
        process.terminate()
    if process.poll() is None:
        try:
            process.wait(timeout=grace_seconds)
        except subprocess.TimeoutExpired:
            process.kill()


def run_process(
    command: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
    timeout_seconds: float,
    output_limit_bytes: int = 1024 * 1024,
    terminate_grace_seconds: float = 0.2,
) -> ProcessOutcome:
    if not command:
        raise ValueError("command cannot be empty")
    if timeout_seconds <= 0:
        raise ValueError("timeout_seconds must be positive")
    start = time.monotonic()
    popen_kwargs: dict[str, object] = {}
    if os.name == "posix":
        popen_kwargs["start_new_session"] = True
    elif os.name == "nt":
        popen_kwargs["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
    process = subprocess.Popen(
        command,
        cwd=str(cwd),
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        **popen_kwargs,
    )
    assert process.stdout is not None and process.stderr is not None
    stdout = _CappedReader(process.stdout, output_limit_bytes)
    stderr = _CappedReader(process.stderr, output_limit_bytes)
    threads = [threading.Thread(target=stdout.run, daemon=True), threading.Thread(target=stderr.run, daemon=True)]
    for thread in threads:
        thread.start()
    timed_out = False
    pgid = process.pid if os.name == "posix" else None
    try:
        process.wait(timeout=timeout_seconds)
    except subprocess.TimeoutExpired:
        timed_out = True
        terminate_process_group(process, pgid, terminate_grace_seconds)
        process.wait()
    except BaseException:
        terminate_process_group(process, pgid, terminate_grace_seconds)
        process.wait()
        raise
    else:
        # The leader may exit while a background child remains in the owned group.
        terminate_process_group(process, pgid, terminate_grace_seconds)
    for thread in threads:
        thread.join(timeout=max(1.0, terminate_grace_seconds + 0.5))
    duration_ms = max(0, int((time.monotonic() - start) * 1000))
    return ProcessOutcome(
        returncode=process.returncode,
        duration_ms=duration_ms,
        stdout=bytes(stdout.data),
        stderr=bytes(stderr.data),
        stdout_truncated=stdout.truncated,
        stderr_truncated=stderr.truncated,
        timed_out=timed_out,
    )


def parse_metrics_artifacts(artifacts: ArtifactSet) -> AgentMetrics:
    """Parse the last `JCODE_EVAL_METRICS {json}` marker from bounded artifacts."""
    data: dict[str, int] = {}
    for path in (artifacts.metrics, artifacts.stderr, artifacts.stdout):
        if path is None or not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        candidates = [text] if path == artifacts.metrics else re.findall(r"JCODE_EVAL_METRICS\s+(\{[^\n]+\})", text)
        for candidate in candidates:
            try:
                parsed = json.loads(candidate)
            except json.JSONDecodeError:
                continue
            if isinstance(parsed, dict):
                data.update({key: int(value) for key, value in parsed.items() if isinstance(value, int) and value >= 0})
    allowed = {field: data.get(field, 0) for field in AgentMetrics.__dataclass_fields__}
    return AgentMetrics(**allowed)
