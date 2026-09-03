from pathlib import Path
raise SystemExit(0 if Path("solution.txt").read_text(encoding="utf-8").strip() == "ok" else 1)
