from __future__ import annotations

from pathlib import Path

from .base import ExplicitBinaryAdapter, ProbeResult, RunSpec


def _binary_contains(path: Path, marker: bytes) -> bool:
    overlap = b""
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            data = overlap + chunk
            if marker in data:
                return True
            overlap = data[-max(0, len(marker) - 1) :]
    return False


class JcodeAdapter(ExplicitBinaryAdapter):
    name = "jcode"
    repo = "1jehuang/jcode"
    capabilities = ("read", "edit", "shell", "lsp", "debugger", "swarm")

    def probe(self) -> ProbeResult:
        base = super().probe()
        if not base.supported or self.binary is None:
            return base
        capabilities = self.capabilities
        if not _binary_contains(self.binary, b"step_in_targets"):
            capabilities = tuple(item for item in capabilities if item != "debugger")
        return ProbeResult(True, capabilities=capabilities)

    def build_command(self, run: RunSpec) -> list[str]:
        if self.binary is None:
            raise RuntimeError("jcode binary path is required")
        socket = run.run_dir / "jcode.sock"
        command = [str(self.binary), "run", "--no-update", "--socket", str(socket)]
        if run.provider:
            command.extend(["--provider", run.provider])
        command.append(run.prompt)
        return command

    def environment(self, run: RunSpec) -> dict[str, str]:
        socket = run.run_dir / "jcode.sock"
        return {
            "JCODE_HOME": str(run.home),
            "JCODE_SOCKET": str(socket),
            "JCODE_TELEMETRY_DISABLED": "1",
            "JCODE_EVAL_DISABLE_MEMORY": "1",
            "JCODE_EVAL_MODEL": run.model or "",
            "JCODE_EVAL_REASONING_EFFORT": run.reasoning_effort or "",
            "JCODE_EVAL_SERVICE_TIER": run.service_tier or "",
        }
