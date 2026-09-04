from __future__ import annotations

from .base import ExplicitBinaryAdapter, RunSpec


class JcodeAdapter(ExplicitBinaryAdapter):
    name = "jcode"
    repo = "1jehuang/jcode"
    capabilities = ("read", "edit", "shell", "swarm")

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
