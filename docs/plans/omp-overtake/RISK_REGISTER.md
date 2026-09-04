# Risk register

| Risk | Mitigation | Gate |
|---|---|---|
| Evaluator overfitting | Lock fixture and verifier digests before implementation | Baseline lock verifies |
| Secret leakage | Allowlist environment capture and redact artifacts | Redaction tests |
| Orphan processes | New process group per trial and group termination | Timeout/interruption tests |
| Cross-trial contamination | Fresh home, runtime, workspace, and socket | Isolation test |
| Invalid partial results | Atomic JSON writes and per-trial result files | Crash test |
| Biased ordering | Seeded per-task randomization | Order test |
| Output exhaustion | Bounded captures with truncation markers | Output cap test |
| Unsupported capability hidden | Explicit unsupported status | Mock campaign |
