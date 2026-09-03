from __future__ import annotations

import json
import sys
from pathlib import Path


def write_fixture(root: Path, behavior: str, *, required_capabilities: list[str] | None = None) -> Path:
    fixture = root / f"fixture-{behavior}"
    fixture.mkdir(parents=True)
    (fixture / "prompt.md").write_text(f"Run deterministic {behavior} fixture.\n", encoding="utf-8")
    (fixture / "verify.py").write_text(
        """from pathlib import Path
import sys

expected = sys.argv[1]
actual = Path("solution.txt").read_text(encoding="utf-8").strip() if Path("solution.txt").exists() else ""
raise SystemExit(0 if actual == expected else 1)
""",
        encoding="utf-8",
    )
    manifest = {
        "id": f"mock-{behavior}",
        "category": "deterministic",
        "description": f"Exercise the mock {behavior} outcome.",
        "fixture": {"source": str(fixture)},
        "prompt_file": "prompt.md",
        "verifier": {
            "command": f'{sys.executable} verify.py ok',
            "timeout_seconds": 2,
        },
        "agent": {
            "timeout_seconds": 1,
            "required_capabilities": required_capabilities or [],
        },
        "expected": {"exit_code": 0, "forbidden_side_effects": []},
        "tags": [f"mock:{behavior}"],
    }
    path = root / f"{behavior}.task.json"
    path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return path
