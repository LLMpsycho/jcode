# Phase 0 acceptance audit

Date: 2026-09-03
Branch: `feat/omp-eval-foundation`

## Interpretation

The request said to implement the master plan. The plan itself says **“Start with Phase 0 only”** and prohibits Phase 1 until the Phase 0 exit gate passes. I therefore interpreted the immediate implementation scope as Phase 0, not all eight phases. This was inferred after work began. If “implement” meant the entire multi-phase program in one turn, the delivered scope is incomplete by design and the plan’s own gates prevent continuing honestly.

## Observed evidence

- `python3 -m unittest discover -s scripts/competitive_eval/tests -v`: 29/29 passed.
- A complete mock campaign continued through pass, fail, timeout, crash, unsupported, and large-output trials. Observed statuses were 2 pass, 1 fail, 1 timeout, 1 crash, and 1 unsupported.
- Timeout tests observed termination of the parent and its spawned child process group.
- Isolation tests observed distinct workspace, home, and temporary directories for each trial.
- Redaction tests observed removal of sensitive environment values, bearer tokens, credential URLs, and key assignments.
- Baseline-lock validation observed all six locked file hashes unchanged after implementation.
- A pinned three-attempt Jcode/OMP dry-run succeeded with Jcode SHA `f46f9c354`, Jcode binary SHA-256 `9a99e3a4800e0275a410020b26479b4a5d76abe6d119fbb76bbf7fff9a627494`, OMP version `18.1.6`, and OMP binary SHA-256 `8a74b820ea220baacffcff22aa740ce09a44f0d5c484b02f2309adf16ad3334c`.

## Requirement traceability

| Phase 0 requirement | Status | Evidence or gap |
|---|---|---|
| Every subprocess has a deadline | Proven | Agent, setup, and verifier calls route through `run_process`; timeout tests pass. |
| Owned process-group termination | Proven on POSIX | Parent and descendant cleanup tests pass on macOS. Windows uses `taskkill` but was not exercised here. |
| Interruption leaves no worker behind | Partially proven | `run_process` cleans up on `BaseException`; campaign writes accumulated results on `KeyboardInterrupt`. No end-to-end SIGINT campaign test exists. |
| Result files remain valid after interruption | Partially proven | Atomic JSON writer and interruption paths exist; atomic writer tests pass. No real signal-injection campaign test exists. |
| Fixture trees are bounded | Proven | File and byte limits plus symlink rejection are tested. |
| Large outputs are capped | Proven | Concurrent capped readers avoid pipe deadlock; truncation is tested. |
| Trials cannot share home/session/memory | Proven for configured paths | Fresh home/temp/workspace/socket paths are tested. Provider-specific hidden global state cannot be ruled out without live runs. |
| One failing trial does not abort campaign | Proven | Mixed mock campaign completed all six trials. |
| No provider secret in artifacts | Proven for tested patterns | Environment allowlisted output is redacted and redaction tests pass. Live-provider artifact review remains pending. |
| Dry-run works without binaries | Proven | Jcode/OMP dry-run test passes without competitor binaries. |
| Mock pass/fail/timeout/crash/unsupported | Proven | All statuses observed in tests and smoke campaign. |
| Comparable Jcode and OMP results | Not proven | Only adapter probing and dry-run planning were executed. |
| Binary/task/verifier hashes recorded | Proven for dry-run | Campaign metadata contains both binary hashes and task/verifier hashes. OMP Git SHA remains `unknown`. |
| Redacted paired baseline report | Not complete | No live paired campaign was run. |
| Cost ceilings enforced | Missing | Task manifests can describe `max_cost_usd`, but the runner does not enforce provider cost ceilings. Live paid campaigns must not run unattended until fixed or externally capped. |
| Forbidden side effects enforced | Missing | `files_changed` is recorded inside the workspace, but `outside_workspace_write` is not deterministically detected. |
| Resource metrics | Incomplete | `peak_rss_bytes` currently remains zero. Startup and memory regression floors are therefore not evaluated by this runner. |

## Comparative conclusion

The evidence proves that the new evaluation foundation is more reliable than having no locked, isolated competitive runner. It does **not** prove that Jcode is better than OMP as a coding agent. A valid overtake claim still requires a live paired campaign on locked tasks, at least three repetitions per task, deterministic verifier outcomes, token and latency collection, resource measurements, safety floors, and a redacted report.

## Gate decision

Phase 0 implementation is usable but its exit gate is not closed. Phase 1 must remain blocked until the missing cost/side-effect/resource controls are addressed as required for the selected campaign and a pinned live Jcode/OMP baseline is completed.
