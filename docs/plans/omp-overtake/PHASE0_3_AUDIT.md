# Bounded Phase 0–3 audit after Phase 4 implementation

Date: 2026-09-04. Base: `LLMpsycho/jcode:master` at
`08e4f14671d23c4b718c2abd656263bba67fa356` (including the DAP follow-ups).

This audit compares the scoped phase records with the corresponding master-plan
requirements. It is a source review plus focused deterministic regressions, not a
new live comparative campaign or proof of every platform/resource acceptance
floor. No locked evaluator file, fixture, threshold, or baseline hash was changed.

## Findings and disposition

| Phase | Observed implementation | Missed, incomplete, or narrower requirement | Disposition in this PR |
| --- | --- | --- | --- |
| 0 | The isolated Python runner, manifest/schema checks, redaction, capped output, timeout/process-group cleanup, atomic result writes and mixed-status campaign behavior remain present. All 32 current tests pass locally; the six-file baseline lock validates unchanged. | `agent.max_cost_usd` is descriptive, not enforced. `forbidden_side_effects` does not enforce/detect writes outside the workspace. `ProcessOutcome.peak_rss_bytes` remains zero. A real signal-injection campaign test and pinned live paired baseline are still absent. | Large runner/containment/measurement work deferred. The older audit's open gate is still valid; later feature completion does not close it retroactively. |
| 1 | `FileSnapshotLedger`, `FileWriteGuard`, all five legacy mutation adapters, `anchored_edit`, strong digest/preflight, exact read coverage, revision metadata, guarded LSP transactions, and staged anchored-edit rollback are present. | Legacy multi-file publication is still sequential after common preflight; an I/O failure can leave partial publication, as `PHASE1_WRITE_GUARD.md` already states. No automatic selfdev/unattended switch from `warn` to `block` was found in the guard policy path. The optional offset-recovery policy is not implemented. Token/RAM/startup and live comparative acceptance remain unmeasured. | These are larger policy/transaction/evaluation slices, not added here. Strict anchored rejection remains the safe behavior without optional recovery. Existing exact-edit behavior is retained. |
| 2 | Shared workspace/worktree pool, bounded client/transport, controlled server environment, fake-server tests, document synchronization, diagnostics, guarded symbol/file rename and code-action apply remain present. | Project/session config layers, restoration beyond clean LSP restart, typed SDK presentation, live Go validation and live paired measurements remain open as documented. In touched diagnostic output, incoming ordering could retain hints while truncating later errors; truncation also erased advisor diagnostic metadata. | **Small gap fixed:** prioritize highest severity before rendering/capping; retain only bounded range/severity/redacted-message metadata, excluding raw server data, even when diagnostic text is truncated. Add `advisor_diagnostics_keep_errors_before_truncated_hints_and_omit_raw_data`. Other items deferred. |
| 3 | The completed 30A–30G and selected post-MVP boundaries are present: shared owner-scoped service, 18-action adapter, launch/owned attach, control, bounded inspection/evaluation, step-in targets, native GDB profile, teardown/reconnect fencing and SDK metadata. | No new missing MVP implementation was identified in this bounded audit. Arbitrary PID attach, reverse process execution, downloads, durable debugger recovery and broader discovery remain intentionally deferred. Live LLDB/GDB/platform acceptance was not rerun here. The Windows follow-up wording was stale: current launch/attach fail closed, rather than exposing a launch with only direct-child containment. | Correct the Windows scope description. Keep authority-expanding items deferred. Capability metadata conservatively treats evaluation and debugger mutations as effects while keeping explicit inspection actions available. |

## Evidence paths

- Phase 0: `scripts/competitive_eval/{run_campaign,run_one,process_metrics}.py`,
  `scripts/competitive_eval/tests`, `competitive-eval/baselines/baseline.lock.json`,
  and `PHASE_0_AUDIT.md`.
- Phase 1: `crates/jcode-edit-core`,
  `crates/jcode-app-core/src/server/file_snapshot_ledger.rs`,
  `tool/file_write_guard.rs`, `tool/file_write_guard_tests.rs`,
  `tool/anchored_edit.rs`, and `tool/anchored_edit/tests.rs`.
- Phase 2: `crates/jcode-lsp`, `crates/jcode-lsp-types`,
  `crates/jcode-app-core/src/tool/lsp.rs`, its `lsp/` helpers/tests and transaction
  integration. Existing real-server tests conditionally depend on installed
  language servers; deterministic success is not a new live Go result.
- Phase 3: `crates/jcode-dap`, `crates/jcode-dap-types`,
  `crates/jcode-app-core/src/tool/dap.rs` (the shared `DapService` and tool),
  lifecycle wiring, and
  the architecture/configuration/tool/troubleshooting documents linked from
  `PHASE3_DAP.md`.

## Validation and repository gates

Local verification:

```text
python3 -m unittest discover -s scripts/competitive_eval/tests -v
32 tests passed (including unchanged baseline-lock verification).
```

The focused PR workflow also runs the edit-core, LSP and DAP library suites,
LSP/DAP agent-tool suites, dependency boundaries, all advisor regressions, a
selfdev build and isolated public-socket acceptance. Refer to the workflow run
for the exact revision and counts; no unexecuted suite is claimed as passing.
A Rust toolchain is not installed in this local workspace, so Rust execution is
performed by GitHub Actions rather than inferred from source inspection.

Existing repository-level gates were checked separately:

- `.github/workflows/ci.yml` has duplicate top-level `env` mappings on the pinned
  base. This PR adds a focused valid workflow; it does not silently repair or
  claim success for the unrelated global workflow.
- Both code-size and test-size ratchets already fail on pinned master (75 and
  30 output lines respectively). Some existing large files necessarily grow to
  expose the stable acceptance DTO/config/capability contracts. The new advisor
  production module is below 1,200 lines after moving its existing tests into a
  test module; new helpers and the touched LSP adapter remain below that limit.
  Broad baseline repair or unrelated extraction is deferred, with no rebaseline.
- Existing test-build unused-import warnings in `server/debug_command_exec.rs`
  and `server/provider_control_tests.rs` are outside the changed behavior. No
  repository-wide strict-clippy success is claimed.
- The PR's linked-issue workflow fails, while this repository has Issues disabled.
  GitHub rejected issue creation with HTTP 410. No repository setting or gate
  was changed to bypass that conflict.

No performance comparison, full multi-platform acceptance, live-model Phase 4
verdict, or Jcode-over-OMP claim is supported by this audit. Those remain separate
acceptance/evaluation work, and Phases 5–7 were not started.
