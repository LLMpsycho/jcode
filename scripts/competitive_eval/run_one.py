from __future__ import annotations

import argparse
import hashlib
import json
import os
import shlex
import shutil
import sys
import time
import uuid
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

if __package__:
    from .adapters.base import AgentAdapter, ArtifactSet, RunSpec
    from .adapters.jcode import JcodeAdapter
    from .adapters.mock import MockAdapter
    from .adapters.omp import OmpAdapter
    from .manifest import load_task_manifest, validate_manifest
    from .process_metrics import ProcessOutcome, run_process
    from .redact import redact_mapping, redact_text, sensitive_values
else:
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
    from scripts.competitive_eval.adapters.base import AgentAdapter, ArtifactSet, RunSpec
    from scripts.competitive_eval.adapters.jcode import JcodeAdapter
    from scripts.competitive_eval.adapters.mock import MockAdapter
    from scripts.competitive_eval.adapters.omp import OmpAdapter
    from scripts.competitive_eval.manifest import load_task_manifest, validate_manifest
    from scripts.competitive_eval.process_metrics import ProcessOutcome, run_process
    from scripts.competitive_eval.redact import redact_mapping, redact_text, sensitive_values


@dataclass(frozen=True)
class TrialConfig:
    campaign_id: str
    attempt: int
    run_dir: Path
    provider: str | None = None
    model: str | None = None
    reasoning_effort: str | None = None
    service_tier: str | None = None
    output_limit_bytes: int = 1024 * 1024
    max_fixture_files: int = 10_000
    max_fixture_bytes: int = 256 * 1024 * 1024


@dataclass(frozen=True)
class TrustedVerifier:
    arguments: tuple[str, ...]
    script_index: int
    relative_path: Path
    content: bytes
    mode: int

    def materialize(self, run_dir: Path) -> list[str]:
        script = run_dir / "trusted-verifier" / self.relative_path
        script.parent.mkdir(parents=True, exist_ok=True)
        script.write_bytes(self.content)
        script.chmod(self.mode)
        arguments = list(self.arguments)
        arguments[self.script_index] = str(script)
        return arguments


def atomic_write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.parent / f".{path.name}.{os.getpid()}.{uuid.uuid4().hex}.tmp"
    try:
        with temporary.open("w", encoding="utf-8") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        if hasattr(os, "O_DIRECTORY"):
            directory_fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(directory_fd)
            finally:
                os.close(directory_fd)
    finally:
        temporary.unlink(missing_ok=True)


def _resolve_inside(root: Path, relative: str, label: str) -> Path:
    candidate = (root / relative).resolve()
    try:
        candidate.relative_to(root.resolve())
    except ValueError as error:
        raise ValueError(f"{label} escapes its root: {relative}") from error
    return candidate


def _bounded_copy(source: Path, destination: Path, max_files: int, max_bytes: int) -> None:
    if not source.is_dir():
        raise ValueError(f"fixture source is not a directory: {source}")
    files = 0
    size = 0
    for path in source.rglob("*"):
        if path.is_symlink():
            raise ValueError(f"fixture symlinks are unsupported in isolated Phase 0 runs: {path}")
        if path.is_file():
            files += 1
            size += path.stat().st_size
            if files > max_files:
                raise ValueError(f"fixture file limit exceeded: {files} > {max_files}")
            if size > max_bytes:
                raise ValueError(f"fixture byte limit exceeded: {size} > {max_bytes}")
    shutil.copytree(source, destination, symlinks=True)


def _snapshot_files(workspace: Path) -> dict[str, str]:
    snapshot: dict[str, str] = {}
    for path in workspace.rglob("*"):
        if path.is_file() and not path.is_symlink():
            snapshot[str(path.relative_to(workspace))] = hashlib.sha256(path.read_bytes()).hexdigest()
    return snapshot


def _snapshot_verifier(workspace: Path, command: str) -> TrustedVerifier:
    arguments = shlex.split(command)
    if not arguments:
        raise ValueError("verifier command cannot be empty")
    root = workspace.resolve()
    for index, argument in enumerate(arguments):
        candidate = Path(argument)
        if not candidate.is_absolute():
            candidate = root / candidate
        try:
            candidate = candidate.resolve()
            relative = candidate.relative_to(root)
        except (OSError, ValueError):
            continue
        if candidate.is_file() and not candidate.is_symlink():
            return TrustedVerifier(
                tuple(arguments),
                index,
                relative,
                candidate.read_bytes(),
                candidate.stat().st_mode & 0o777,
            )
    raise ValueError("verifier command must reference a workspace-local regular file")


def _write_artifact(path: Path, data: bytes, secrets: list[str], limit: int) -> None:
    text = data.decode("utf-8", errors="replace")
    redacted = redact_text(text, secrets).encode("utf-8")[: max(0, limit)]
    path.write_text(redacted.decode("utf-8", errors="ignore"), encoding="utf-8")


def _isolated_environment(run: RunSpec, adapter: AgentAdapter) -> tuple[dict[str, str], dict[str, str], list[str]]:
    environment = os.environ.copy()
    isolation = {
        "HOME": str(run.home),
        "TMPDIR": str(run.temp_dir),
        "TMP": str(run.temp_dir),
        "TEMP": str(run.temp_dir),
        "XDG_CONFIG_HOME": str(run.home / ".config"),
        "XDG_CACHE_HOME": str(run.home / ".cache"),
        "XDG_DATA_HOME": str(run.home / ".local" / "share"),
        "JCODE_EVAL_CAMPAIGN_ID": run.campaign_id,
        "JCODE_EVAL_TASK_ID": run.task_id,
        "JCODE_EVAL_ATTEMPT": str(run.attempt),
        "JCODE_EVAL_WORKSPACE": str(run.workspace),
    }
    if "RUSTUP_HOME" not in environment:
        original_home = environment.get("HOME")
        if original_home:
            default_rustup_home = Path(original_home) / ".rustup"
            if default_rustup_home.is_dir():
                isolation["RUSTUP_HOME"] = str(default_rustup_home)
    overrides = {**isolation, **adapter.environment(run)}
    environment.update(overrides)
    secrets = sensitive_values(environment)
    return environment, overrides, secrets


def _empty_result(config: TrialConfig, task: dict[str, Any], adapter: AgentAdapter, start_time: str) -> dict[str, Any]:
    version = adapter.version()
    return {
        "campaign_id": config.campaign_id,
        "task_id": task["id"],
        "agent": adapter.name,
        "agent_git_sha": version.git_sha,
        "model": config.model,
        "provider": config.provider,
        "attempt": config.attempt,
        "start_time": start_time,
        "duration_ms": 0,
        "status": "error",
        "verifier_exit_code": None,
        "tool_calls": 0,
        "tool_failures": 0,
        "edit_calls": 0,
        "edit_retries": 0,
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "peak_rss_bytes": 0,
        "human_interventions": 0,
        "files_changed": [],
        "stdout_artifact": None,
        "stderr_artifact": None,
        "transcript_artifact": None,
        "failure_class": None,
        "output_truncated": False,
    }


def run_trial(
    task: dict[str, Any],
    *,
    manifest_path: Path,
    adapter: AgentAdapter,
    config: TrialConfig,
) -> dict[str, Any]:
    validate_manifest(task, "task")
    started = time.monotonic()
    start_time = datetime.now(timezone.utc).isoformat()
    run_dir = config.run_dir.resolve()
    run_dir.mkdir(parents=True, exist_ok=True)
    result = _empty_result(config, task, adapter, start_time)
    result_path = run_dir / "result.json"
    workspace = run_dir / "workspace"
    home = run_dir / "home"
    temp_dir = run_dir / "tmp"
    home.mkdir()
    temp_dir.mkdir()
    source_value = task["fixture"]["source"]
    source = Path(source_value)
    if not source.is_absolute():
        source = manifest_path.resolve().parent / source
    source = source.resolve()
    _bounded_copy(source, workspace, config.max_fixture_files, config.max_fixture_bytes)
    result["workspace"] = str(workspace)
    initial_files = _snapshot_files(workspace)
    prompt_file = _resolve_inside(workspace, task["prompt_file"], "prompt_file")
    if not prompt_file.is_file():
        raise ValueError(f"prompt file does not exist: {prompt_file}")
    trusted_verifier = _snapshot_verifier(workspace, task["verifier"]["command"])
    run = RunSpec(
        campaign_id=config.campaign_id,
        task_id=task["id"],
        attempt=config.attempt,
        workspace=workspace,
        home=home,
        temp_dir=temp_dir,
        run_dir=run_dir,
        prompt_file=prompt_file,
        prompt=prompt_file.read_text(encoding="utf-8"),
        timeout_seconds=task["agent"]["timeout_seconds"],
        provider=config.provider,
        model=config.model,
        reasoning_effort=config.reasoning_effort,
        service_tier=config.service_tier,
        required_capabilities=tuple(task["agent"].get("required_capabilities", [])),
        tags=tuple(task.get("tags", [])),
    )
    environment, captured_environment, secrets = _isolated_environment(run, adapter)
    atomic_write_json(run_dir / "environment.json", redact_mapping(captured_environment, secrets))
    probe = adapter.probe()
    missing = sorted(set(run.required_capabilities) - set(probe.capabilities))
    if not probe.supported or missing:
        result["status"] = "unsupported"
        result["failure_class"] = "unsupported_capability" if missing else "adapter_unavailable"
        result["unsupported_reason"] = f"missing capabilities: {', '.join(missing)}" if missing else probe.reason
        result["duration_ms"] = int((time.monotonic() - started) * 1000)
        validate_manifest(result, "result")
        atomic_write_json(result_path, result)
        return result

    outcome: ProcessOutcome | None = None
    try:
        setup = task.get("setup")
        if setup and setup.get("command"):
            setup_outcome = run_process(
                shlex.split(setup["command"]), cwd=workspace, env=environment,
                timeout_seconds=min(60, run.timeout_seconds), output_limit_bytes=config.output_limit_bytes,
            )
            _write_artifact(run_dir / "setup_stdout.log", setup_outcome.stdout, secrets, config.output_limit_bytes)
            _write_artifact(run_dir / "setup_stderr.log", setup_outcome.stderr, secrets, config.output_limit_bytes)
            if setup_outcome.timed_out or setup_outcome.returncode != 0:
                result["status"] = "error"
                result["failure_class"] = "setup_timeout" if setup_outcome.timed_out else "setup_exit"
                result["duration_ms"] = int((time.monotonic() - started) * 1000)
                validate_manifest(result, "result")
                atomic_write_json(result_path, result)
                return result
        outcome = run_process(
            adapter.build_command(run), cwd=workspace, env=environment,
            timeout_seconds=run.timeout_seconds, output_limit_bytes=config.output_limit_bytes,
        )
        stdout_path = run_dir / "stdout.log"
        stderr_path = run_dir / "stderr.log"
        _write_artifact(stdout_path, outcome.stdout, secrets, config.output_limit_bytes)
        _write_artifact(stderr_path, outcome.stderr, secrets, config.output_limit_bytes)
        result["stdout_artifact"] = stdout_path.name
        result["stderr_artifact"] = stderr_path.name
        result["output_truncated"] = outcome.stdout_truncated or outcome.stderr_truncated
        result["peak_rss_bytes"] = outcome.peak_rss_bytes
        if outcome.timed_out:
            result["status"] = "timeout"
            result["failure_class"] = "agent_timeout"
        elif outcome.returncode != 0:
            result["status"] = "crash"
            result["failure_class"] = "agent_exit"
            result["agent_exit_code"] = outcome.returncode
        else:
            verifier = task["verifier"]
            verifier_outcome = run_process(
                trusted_verifier.materialize(run_dir), cwd=workspace, env=environment,
                timeout_seconds=verifier["timeout_seconds"], output_limit_bytes=config.output_limit_bytes,
            )
            _write_artifact(run_dir / "verifier_stdout.log", verifier_outcome.stdout, secrets, config.output_limit_bytes)
            _write_artifact(run_dir / "verifier_stderr.log", verifier_outcome.stderr, secrets, config.output_limit_bytes)
            result["verifier_exit_code"] = verifier_outcome.returncode
            if verifier_outcome.timed_out:
                result["status"] = "timeout"
                result["failure_class"] = "verifier_timeout"
            elif verifier_outcome.returncode == task["expected"]["exit_code"]:
                result["status"] = "pass"
            else:
                result["status"] = "fail"
                result["failure_class"] = "verifier"
        if result["stdout_artifact"] and result["stderr_artifact"]:
            metrics = adapter.parse_metrics(ArtifactSet(run_dir / "stdout.log", run_dir / "stderr.log"))
            result.update(asdict(metrics))
    except BaseException as error:
        result["status"] = "error"
        result["failure_class"] = "runner_interrupted" if isinstance(error, KeyboardInterrupt) else "runner_error"
        result["runner_error"] = redact_text(str(error), secrets)
        result["duration_ms"] = int((time.monotonic() - started) * 1000)
        result["files_changed"] = sorted(set(_snapshot_files(workspace)) ^ set(initial_files))
        validate_manifest(result, "result")
        atomic_write_json(result_path, result)
        raise
    final_files = _snapshot_files(workspace)
    result["files_changed"] = sorted(
        path for path in set(initial_files) | set(final_files) if initial_files.get(path) != final_files.get(path)
    )
    result["duration_ms"] = int((time.monotonic() - started) * 1000)
    validate_manifest(result, "result")
    atomic_write_json(result_path, result)
    return result


def _adapter(name: str, binary: str | None) -> AgentAdapter:
    if name == "mock":
        return MockAdapter()
    if name == "jcode":
        return JcodeAdapter(binary)
    if name == "omp":
        return OmpAdapter(binary)
    raise ValueError(name)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run one isolated competitive evaluation trial.")
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--agent", choices=("mock", "jcode", "omp"), default="mock")
    parser.add_argument("--binary")
    parser.add_argument("--campaign-id", default="manual")
    parser.add_argument("--attempt", type=int, default=1)
    parser.add_argument("--result-dir", type=Path, required=True)
    parser.add_argument("--output-limit-bytes", type=int, default=1024 * 1024)
    args = parser.parse_args(argv)
    result = run_trial(
        load_task_manifest(args.manifest), manifest_path=args.manifest, adapter=_adapter(args.agent, args.binary),
        config=TrialConfig(args.campaign_id, args.attempt, args.result_dir, output_limit_bytes=args.output_limit_bytes),
    )
    print(json.dumps(result, sort_keys=True))
    return 0 if result["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
