from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import random
import re
from collections.abc import Sequence
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .adapters.base import AgentAdapter
from .adapters.jcode import JcodeAdapter
from .adapters.mock import MockAdapter
from .adapters.omp import OmpAdapter
from .manifest import load_task_manifest, validate_baseline_lock, validate_manifest
from .run_one import TrialConfig, atomic_write_json, run_trial
from .redact import redact_text, sensitive_values


REPO_ROOT = Path(__file__).resolve().parents[2]


def build_trial_plan(
    task_ids: Sequence[str],
    agents: Sequence[str],
    *,
    attempts: int,
    seed: int,
) -> list[dict[str, Any]]:
    if attempts < 1:
        raise ValueError("attempts must be at least 1")
    if not agents:
        raise ValueError("at least one agent is required")
    generator = random.Random(seed)
    plan: list[dict[str, Any]] = []
    for task_id in task_ids:
        for attempt in range(1, attempts + 1):
            order = list(agents)
            generator.shuffle(order)
            plan.append({"task_id": task_id, "attempt": attempt, "agents": order})
    return plan


def _combined_hash(paths: Sequence[Path]) -> str:
    digest = hashlib.sha256()
    for path in sorted((item.resolve() for item in paths), key=str):
        digest.update(str(path).encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def _verifier_hash(manifests: Sequence[tuple[Path, dict[str, Any]]]) -> str:
    digest = hashlib.sha256()
    for path, task in sorted(manifests, key=lambda item: item[1]["id"]):
        verifier = task["verifier"]
        digest.update(json.dumps(verifier, sort_keys=True, separators=(",", ":")).encode("utf-8"))
        source = Path(task["fixture"]["source"])
        if not source.is_absolute():
            source = (path.parent / source).resolve()
        for token in verifier["command"].split():
            candidate = (source / token).resolve()
            if candidate.is_file():
                try:
                    candidate.relative_to(source)
                except ValueError:
                    continue
                digest.update(candidate.read_bytes())
    return digest.hexdigest()


def _campaign_id() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")


def _adapter_map(jcode_binary: Path | None, omp_binary: Path | None) -> dict[str, AgentAdapter]:
    return {
        "mock": MockAdapter(),
        "jcode": JcodeAdapter(jcode_binary),
        "omp": OmpAdapter(omp_binary),
    }


def _host_info() -> dict[str, Any]:
    return {
        "os": platform.system().lower(),
        "arch": platform.machine(),
        "cpu": platform.processor() or "unknown",
        "ram_bytes": 0,
    }


def run_campaign(
    manifest_paths: Sequence[Path],
    *,
    agents: Sequence[str],
    attempts: int,
    seed: int,
    results_root: Path,
    campaign_id: str | None = None,
    jcode_binary: Path | None = None,
    omp_binary: Path | None = None,
    provider: str = "deterministic",
    model: str = "mock",
    reasoning_effort: str | None = None,
    service_tier: str | None = None,
    output_limit_bytes: int = 1024 * 1024,
    dry_run: bool = False,
    repo_root: Path = REPO_ROOT,
) -> tuple[Path, list[dict[str, Any]]]:
    validate_baseline_lock(repo_root)
    loaded = [(path.resolve(), load_task_manifest(path)) for path in manifest_paths]
    if not loaded:
        raise ValueError("no task manifests were supplied")
    task_ids = [task["id"] for _, task in loaded]
    if len(task_ids) != len(set(task_ids)):
        raise ValueError("task ids must be unique within a campaign")
    adapters = _adapter_map(jcode_binary, omp_binary)
    unknown_agents = sorted(set(agents) - set(adapters))
    if unknown_agents:
        raise ValueError(f"unknown agents: {', '.join(unknown_agents)}")
    identifier = campaign_id or _campaign_id()
    if not re.fullmatch(r"[A-Za-z0-9._-]+", identifier):
        raise ValueError("campaign_id may contain only letters, numbers, dot, underscore, and dash")
    campaign_dir = (results_root / identifier).resolve()
    campaign_dir.mkdir(parents=True, exist_ok=False)
    plan = build_trial_plan(task_ids, agents, attempts=attempts, seed=seed)
    campaign = {
        "campaign_id": identifier,
        "jcode": adapters["jcode"].version().to_dict(),
        "omp": adapters["omp"].version().to_dict(),
        "provider": provider,
        "model": model,
        "reasoning_effort": reasoning_effort,
        "service_tier": service_tier,
        "seed": seed,
        "host": _host_info(),
        "task_manifest_sha256": _combined_hash([path for path, _ in loaded]),
        "verifier_sha256": _verifier_hash(loaded),
        "attempts": attempts,
        "agents": list(agents),
        "dry_run": dry_run,
    }
    validate_manifest(campaign, "campaign")
    atomic_write_json(campaign_dir / "campaign.json", campaign)
    atomic_write_json(campaign_dir / "plan.json", plan)
    if dry_run:
        return campaign_dir, []

    tasks = {task["id"]: (path, task) for path, task in loaded}
    results: list[dict[str, Any]] = []
    for plan_item in plan:
        manifest_path, task = tasks[plan_item["task_id"]]
        for order, agent_name in enumerate(plan_item["agents"], start=1):
            run_dir = campaign_dir / "trials" / task["id"] / f"attempt-{plan_item['attempt']}" / f"{order:02d}-{agent_name}"
            try:
                result = run_trial(
                    task,
                    manifest_path=manifest_path,
                    adapter=adapters[agent_name],
                    config=TrialConfig(
                        identifier,
                        plan_item["attempt"],
                        run_dir,
                        provider=provider,
                        model=model,
                        reasoning_effort=reasoning_effort,
                        service_tier=service_tier,
                        output_limit_bytes=output_limit_bytes,
                    ),
                )
            except KeyboardInterrupt:
                atomic_write_json(campaign_dir / "results.json", results)
                raise
            except Exception as error:
                result = {
                    "campaign_id": identifier,
                    "task_id": task["id"],
                    "agent": agent_name,
                    "attempt": plan_item["attempt"],
                    "start_time": datetime.now(timezone.utc).isoformat(),
                    "duration_ms": 0,
                    "status": "error",
                    "tool_calls": 0,
                    "tool_failures": 0,
                    "edit_calls": 0,
                    "edit_retries": 0,
                    "human_interventions": 0,
                    "files_changed": [],
                    "failure_class": "runner_error",
                    "runner_error": redact_text(str(error), sensitive_values(os.environ)),
                }
                validate_manifest(result, "result")
                atomic_write_json(run_dir / "result.json", result)
            results.append(result)
            atomic_write_json(campaign_dir / "results.json", results)
    return campaign_dir, results


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run a deterministic competitive evaluation campaign.")
    parser.add_argument("manifests", nargs="+", type=Path)
    parser.add_argument("--agents", default="mock", help="Comma-separated mock,jcode,omp list")
    parser.add_argument("--attempts", type=int, default=1)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--campaign-id")
    parser.add_argument("--results-root", type=Path, default=REPO_ROOT / "competitive-eval" / "campaigns")
    parser.add_argument("--jcode-binary", type=Path, default=os.environ.get("JCODE_EVAL_JCODE_BINARY"))
    parser.add_argument("--omp-binary", type=Path, default=os.environ.get("JCODE_EVAL_OMP_BINARY"))
    parser.add_argument("--provider", default=os.environ.get("JCODE_EVAL_PROVIDER", "deterministic"))
    parser.add_argument("--model", default=os.environ.get("JCODE_EVAL_MODEL", "mock"))
    parser.add_argument("--reasoning-effort", default=os.environ.get("JCODE_EVAL_REASONING_EFFORT"))
    parser.add_argument("--service-tier", default=os.environ.get("JCODE_EVAL_SERVICE_TIER"))
    parser.add_argument("--output-limit-bytes", type=int, default=1024 * 1024)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)
    campaign_dir, results = run_campaign(
        args.manifests,
        agents=[item.strip() for item in args.agents.split(",") if item.strip()],
        attempts=args.attempts,
        seed=args.seed,
        results_root=args.results_root,
        campaign_id=args.campaign_id,
        jcode_binary=args.jcode_binary,
        omp_binary=args.omp_binary,
        provider=args.provider,
        model=args.model,
        reasoning_effort=args.reasoning_effort,
        service_tier=args.service_tier,
        output_limit_bytes=args.output_limit_bytes,
        dry_run=args.dry_run,
    )
    print(campaign_dir)
    return 0 if args.dry_run or all(item["status"] == "pass" for item in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
