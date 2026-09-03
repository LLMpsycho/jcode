from __future__ import annotations

import argparse
import json
import math
import statistics
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


def load_results(path: Path) -> list[dict[str, Any]]:
    if path.is_file():
        data = json.loads(path.read_text(encoding="utf-8"))
        return data if isinstance(data, list) else [data]
    return [json.loads(item.read_text(encoding="utf-8")) for item in sorted(path.glob("trials/**/result.json"))]


def _p90(values: list[int]) -> int:
    if not values:
        return 0
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * 0.9) - 1)]


def summarize_results(results: list[dict[str, Any]]) -> dict[str, Any]:
    by_agent: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for result in results:
        by_agent[result["agent"]].append(result)
    summary: dict[str, Any] = {"total_trials": len(results), "agents": {}}
    for agent, items in sorted(by_agent.items()):
        statuses = Counter(item["status"] for item in items)
        durations = [int(item.get("duration_ms", 0)) for item in items]
        summary["agents"][agent] = {
            "trials": len(items),
            "statuses": dict(sorted(statuses.items())),
            "success_rate": statuses["pass"] / len(items),
            "median_duration_ms": int(statistics.median(durations)),
            "p90_duration_ms": _p90(durations),
            "output_truncated": sum(bool(item.get("output_truncated")) for item in items),
        }
    return summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Summarize competitive evaluation result JSON.")
    parser.add_argument("path", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    output = json.dumps(summarize_results(load_results(args.path)), indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(output, encoding="utf-8")
    else:
        print(output, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
