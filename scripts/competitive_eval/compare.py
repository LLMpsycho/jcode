from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path
from typing import Any

from .summarize import load_results


def compare_results(results: list[dict[str, Any]], left: str = "jcode", right: str = "omp") -> dict[str, Any]:
    indexed = {(item["task_id"], item["attempt"], item["agent"]): item for item in results}
    pairs = sorted({(task, attempt) for task, attempt, agent in indexed if agent in {left, right}})
    outcomes = Counter()
    details = []
    for task, attempt in pairs:
        left_result = indexed.get((task, attempt, left))
        right_result = indexed.get((task, attempt, right))
        if left_result is None or right_result is None:
            outcome = "unpaired"
        elif left_result["status"] == right_result["status"]:
            outcome = "tie"
        elif left_result["status"] == "pass":
            outcome = f"{left}_win"
        elif right_result["status"] == "pass":
            outcome = f"{right}_win"
        else:
            outcome = "tie_nonpass"
        outcomes[outcome] += 1
        details.append({"task_id": task, "attempt": attempt, "outcome": outcome})
    return {"left": left, "right": right, "paired_attempts": len(pairs), "outcomes": dict(outcomes), "details": details}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Compare paired agent results.")
    parser.add_argument("path", type=Path)
    parser.add_argument("--left", default="jcode")
    parser.add_argument("--right", default="omp")
    args = parser.parse_args(argv)
    print(json.dumps(compare_results(load_results(args.path), args.left, args.right), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
