# Baseline report

Status: deterministic Phase 0 runner complete; paired live baseline pending explicit pinned binaries.

This report intentionally makes no comparative claim until both explicit binaries have completed a pinned paired campaign. Generated artifacts remain local under `competitive-eval/campaigns/`; only curated redacted summaries may be committed.

## Deterministic foundation

- Baseline schemas and governance inputs are SHA-256 locked before implementation.
- The standard-library Python runner supports dry-run, mock, Jcode, and OMP adapters.
- Every trial receives an isolated workspace, home, temporary directory, and runtime/socket path.
- Agent and verifier subprocesses have deadlines, bounded output, and owned process-group cleanup.
- Results are written atomically and campaigns continue after individual failures.
- Artifact and environment redaction is covered by deterministic tests.
- Seeded ordering is reproducible while varying the Jcode/OMP order across paired trials.
- Mock coverage includes pass, fail, timeout, crash, unsupported, and large-output cases.

The deterministic suite currently contains 29 passing tests. The bundled mock campaign records two passes, one failure, one timeout, one crash, and one unsupported result without aborting the campaign.

## Remaining Phase 0 exit work

- Run the first paired campaign with explicit version-pinned Jcode and OMP binaries.
- Record comparable binary, task, prompt, and verifier fingerprints in the campaign artifacts.
- Curate a redacted paired baseline report without changing the locked evaluator inputs.

## Reliability concerns to classify

- Low-signal tool-call thrashing: pending a dedicated deterministic reproduction.
- Recursive listing/output growth: runner output and fixture trees are bounded; product-level reproduction remains pending.
- Hung ambient cycles: campaign-level deadlines prevent an indefinite evaluation wait; product-level reproduction remains pending.
- Worker working-directory/shell failures: trial isolation and working-directory behavior are covered in the runner; product-level reproduction remains pending.
- Task cancellation/retry liveness: owned process-group termination is covered; product-level retry-state reproduction remains pending.
