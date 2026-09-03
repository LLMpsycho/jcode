from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

try:
    from .base import AgentMetrics, ArtifactSet, ProbeResult, RunSpec, VersionInfo
except ImportError:  # Direct execution by the harness.
    from base import AgentMetrics, ArtifactSet, ProbeResult, RunSpec, VersionInfo


class MockAdapter:
    name = "mock"
    capabilities = ("read", "edit", "shell")

    def probe(self) -> ProbeResult:
        return ProbeResult(True, capabilities=self.capabilities)

    def version(self) -> VersionInfo:
        return VersionInfo("local/mock", "deterministic", "0" * 64, "mock-v1")

    def build_command(self, run: RunSpec) -> list[str]:
        behavior = next((tag.split(":", 1)[1] for tag in run.tags if tag.startswith("mock:")), "pass")
        return [sys.executable, str(Path(__file__).resolve()), "--behavior", behavior]

    def environment(self, run: RunSpec) -> dict[str, str]:
        return {"JCODE_EVAL_MOCK_RUN_DIR": str(run.run_dir)}

    def parse_metrics(self, artifacts: ArtifactSet) -> AgentMetrics:
        return AgentMetrics(tool_calls=1, edit_calls=1)

    def terminate(self, process: subprocess.Popen[bytes]) -> None:
        try:
            from ..process_metrics import terminate_process_group
        except ImportError:
            return
        terminate_process_group(process)


def mock_main(behavior: str) -> int:
    if behavior == "timeout":
        subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
        time.sleep(60)
        return 0
    if behavior == "crash":
        print("deterministic mock crash", file=sys.stderr)
        return 17
    if behavior == "large-output":
        sys.stdout.write("o" * 200_000)
        sys.stderr.write("e" * 200_000)
        Path("solution.txt").write_text("ok\n", encoding="utf-8")
        return 0
    Path("solution.txt").write_text("wrong\n" if behavior == "fail" else "ok\n", encoding="utf-8")
    print('JCODE_EVAL_METRICS ' + json.dumps({"tool_calls": 1, "edit_calls": 1}))
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--behavior", required=True)
    args = parser.parse_args(argv)
    return mock_main(args.behavior)


if __name__ == "__main__":
    raise SystemExit(main())
