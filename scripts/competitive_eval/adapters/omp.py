from __future__ import annotations

import os
import shlex

from .base import ExplicitBinaryAdapter, RunSpec


class OmpAdapter(ExplicitBinaryAdapter):
    name = "omp"
    repo = "can1357/oh-my-pi"
    capabilities = ("read", "edit", "shell", "lsp", "debugger", "swarm")

    def build_command(self, run: RunSpec) -> list[str]:
        if self.binary is None:
            raise RuntimeError("OMP binary path is required")
        configured = os.environ.get("JCODE_EVAL_OMP_ARGS")
        args = shlex.split(configured) if configured else ["--print", "{prompt}"]
        substitutions = {
            "{prompt}": run.prompt,
            "{prompt_file}": str(run.prompt_file),
            "{workspace}": str(run.workspace),
        }
        expanded = [substitutions.get(item, item) for item in args]
        return [str(self.binary), *expanded]

    def environment(self, run: RunSpec) -> dict[str, str]:
        return {
            "OMP_HOME": str(run.home),
            "XDG_CONFIG_HOME": str(run.home / ".config"),
            "XDG_CACHE_HOME": str(run.home / ".cache"),
            "OMP_TELEMETRY_DISABLED": "1",
            "OMP_DISABLE_MEMORY": "1",
            "JCODE_EVAL_PROVIDER": run.provider or "",
            "JCODE_EVAL_MODEL": run.model or "",
            "JCODE_EVAL_REASONING_EFFORT": run.reasoning_effort or "",
            "JCODE_EVAL_SERVICE_TIER": run.service_tier or "",
        }
