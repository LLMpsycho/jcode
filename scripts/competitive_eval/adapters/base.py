from __future__ import annotations

import hashlib
import os
import re
import subprocess
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Protocol


@dataclass(frozen=True)
class ProbeResult:
    supported: bool
    reason: str | None = None
    capabilities: tuple[str, ...] = ()


@dataclass(frozen=True)
class VersionInfo:
    repo: str
    git_sha: str
    binary_sha256: str
    version: str

    def to_dict(self) -> dict[str, str]:
        return asdict(self)


@dataclass(frozen=True)
class RunSpec:
    campaign_id: str
    task_id: str
    attempt: int
    workspace: Path
    home: Path
    temp_dir: Path
    run_dir: Path
    prompt_file: Path
    prompt: str
    timeout_seconds: float
    provider: str | None
    model: str | None
    reasoning_effort: str | None
    service_tier: str | None
    required_capabilities: tuple[str, ...]
    tags: tuple[str, ...]


@dataclass(frozen=True)
class ArtifactSet:
    stdout: Path
    stderr: Path
    transcript: Path | None = None
    metrics: Path | None = None


@dataclass(frozen=True)
class AgentMetrics:
    tool_calls: int = 0
    tool_failures: int = 0
    edit_calls: int = 0
    edit_retries: int = 0
    input_tokens: int = 0
    output_tokens: int = 0
    cache_read_tokens: int = 0


class AgentAdapter(Protocol):
    name: str

    def probe(self) -> ProbeResult: ...
    def version(self) -> VersionInfo: ...
    def build_command(self, run: RunSpec) -> list[str]: ...
    def environment(self, run: RunSpec) -> dict[str, str]: ...
    def parse_metrics(self, artifacts: ArtifactSet) -> AgentMetrics: ...
    def terminate(self, process: subprocess.Popen[bytes]) -> None: ...


def binary_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


class ExplicitBinaryAdapter:
    name = "unknown"
    repo = "unknown"
    capabilities: tuple[str, ...] = ("read", "edit", "shell")

    def __init__(self, binary: Path | str | None) -> None:
        self.binary = Path(binary).expanduser().resolve() if binary else None

    def probe(self) -> ProbeResult:
        if self.binary is None:
            return ProbeResult(False, f"{self.name} binary path was not supplied", self.capabilities)
        if not self.binary.is_file():
            return ProbeResult(False, f"{self.name} binary does not exist: {self.binary}", self.capabilities)
        if not os.access(self.binary, os.X_OK):
            return ProbeResult(False, f"{self.name} binary is not executable: {self.binary}", self.capabilities)
        return ProbeResult(True, capabilities=self.capabilities)

    def version(self) -> VersionInfo:
        probe = self.probe()
        if not probe.supported or self.binary is None:
            return VersionInfo(self.repo, "unknown", "0" * 64, probe.reason or "unsupported")
        try:
            completed = subprocess.run(
                [str(self.binary), "--version"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                timeout=5,
                check=False,
                text=True,
            )
            output = completed.stdout.strip()[:4096] or f"exit {completed.returncode}"
        except (OSError, subprocess.TimeoutExpired) as error:
            output = f"version probe failed: {error}"
        sha_match = re.search(r"\b[0-9a-f]{7,40}\b", output, re.IGNORECASE)
        return VersionInfo(self.repo, sha_match.group(0) if sha_match else "unknown", binary_sha256(self.binary), output)

    def parse_metrics(self, artifacts: ArtifactSet) -> AgentMetrics:
        from ..process_metrics import parse_metrics_artifacts

        return parse_metrics_artifacts(artifacts)

    def terminate(self, process: subprocess.Popen[bytes]) -> None:
        from ..process_metrics import terminate_process_group

        terminate_process_group(process)
