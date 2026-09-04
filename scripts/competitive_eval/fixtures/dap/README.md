# DAP fixtures

These deterministic Rust tasks exercise debugger behavior rather than raw edit
throughput:

- `rust-crash-localization` requires launch, breakpoint, stack-frame, and
  variable inspection to diagnose an out-of-bounds panic.
- `rust-step-target-repair` requires `step_in_targets` and targeted `step_in` on
  a nested expression before repairing the incorrect transformation.

Each task's `setup.py` builds a debuggable binary and writes a DAP-only config
under the isolated trial `JCODE_HOME`. It discovers `lldb-dap` from
`JCODE_EVAL_LLDB_DAP`, `PATH`, or `xcrun`, in that order. It does not download
an adapter or alter user configuration.

Validate manifests without invoking a model:

```sh
python3 - <<'PY'
from pathlib import Path
from scripts.competitive_eval.manifest import load_task_manifest
for path in Path("scripts/competitive_eval/fixtures/dap").glob("*/task.json"):
    load_task_manifest(path)
PY
```
