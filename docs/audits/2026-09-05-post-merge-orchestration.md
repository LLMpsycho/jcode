# Post-merge orchestration audit and hardening

Date: 5 September 2026  
Repository: `LLMpsycho/jcode`  
Audited base: `master` at `97107d565165c6ad5ef5ea2d7899a03c44dd1c16`  
Scope: recent advisor / independent-role model changes, connected TUI command routing, automatic review/judge launches, split lifecycle, and workflow startup. The Phase 0–4 records were also checked for explicitly unfinished acceptance work.

## Executive assessment

The recent features are substantial, but the most immediate weakness is integration consistency: one input route works while another falls through to local execution; configuration flags exist without their runtime consumer; several launch flows share mutable state without a common ownership check. Adding more agents before stabilizing these boundaries would amplify failures rather than improve task completion.

This patch completes the missing connected-client review scheduling and hardens the neighboring launch paths. It does not certify the entire repository, turn the TUI scheduler into a durable headless service, or establish that jcode outperforms competing agents.

## What was examined

PR 15 fixes advisor dispatch before skill lookup. PR 14 adds independent model, authenticated route and reasoning effort selection for seven roles and explicitly records the missing remote automatic-review consumer. PR 13 supplies preceding advisor isolation, persistence and picker changes. Their implementation paths and the current source—not only their descriptions—were inspected.

The terminal could not resolve GitHub and had no Rust toolchain. GitHub Actions performed real checkouts, including a full-history checkout for validation. An exact tracked-source archive was downloaded for local inspection and patch construction. Rust validation ran in isolated GitHub runners, not against a developer's live daemon. The work used a new branch; `master` was not modified or merged.

## Fixes in this change

| Priority | Finding and user impact | Resolution |
| --- | --- | --- |
| P1 | Connected review controls had separate Enter and generic-input implementations. Generic submissions could enter local handlers instead of sending the proper remote controls. | One connected dispatcher handles `/review`, `/judge`, `/autoreview` and `/autojudge` before skill dispatch in both paths. Toggles send feature-control requests and do not become main-model prompts. |
| P1 | End-of-turn automatic review/judge flags were armed but never consumed. Enabling the feature did not reliably create the promised agents. | Consume the flags only after accepted successful completion; enqueue at most one review and one judge for the latest eligible turn; launch serially through the existing split handshake. |
| P1 | Review, judge, prompt, transfer, workspace and plain-fork flows could collide over the shared pending split metadata. | Check launch ownership before mutation across those entry points. Preserve the first request's prompt, parent, model, authenticated route and effort. A colliding prompt submission is restored instead of replacing the first launch. |
| P1 | Split control responses were not correlated independently from a main-model turn. A failed split could leave the new collision guard permanently occupied. | Record split request IDs, reject mismatched/stale split replies, release matching failed launches, and avoid resetting an unrelated active main turn. Clear pending workspace split intent on failure. |
| P2 | Reconnecting during an unresolved launch could leave stale client ownership and replay ambiguity. | Clear connection-scoped IDs and automatic work; report an unknown launch outcome rather than automatically retrying a possibly successful split. Request IDs from different sockets are never compared. |
| P2 | Prefix matching let built-in local review commands claim similarly named skills, such as `/reviewer`. | Match the complete first command token. Validate arguments before queuing a launch. |
| P1 | Three workflows contained duplicate top-level `env` mappings, preventing valid workflow startup. Main CI also required an obsolete SSH deploy-key setup despite the inspected checkout/dependencies being public HTTPS with no submodules. | Consolidate environment mappings without dropping variables; remove obsolete SSH-only checkout/dependency setup. Add strict duplicate-key workflow tests and a dedicated read-only regression workflow. Existing quality checks remain in place. |

### Scheduling and lifecycle guarantees—and their limits

Only a client that submitted a user turn arms automatic review. Observer clients, system submissions, stale completions, interrupted turns and provider guardrail failures do not independently multiply review agents. Both role selection and source session are captured before waiting for the split slot. Disabling a queued role prevents its launch. Child review/judge sessions keep automatic review disabled to avoid recursive fan-out.

A new submission or connection/session change can discard unsent automatic work. This is deliberately a bounded, client-originated scheduler, not a persisted daemon job system. A terminal that cannot be opened remains subject to the existing manual-resume fallback. Terminal creation and live paid-provider behavior are not inferred from unit tests.

The split correlation applies to the connected split flows changed here. It is not a replacement for a repository-wide typed request registry covering every feature toggle, transfer, catalog, model-switch and background event. Durable recovery of arbitrary prompt/attachment drafts is also separate work.

## Regression evidence

The candidate passed the following checks in GitHub Actions run [33979300344](https://github.com/LLMpsycho/jcode/actions/runs/33979300344), on Ubuntu with Rust stable 1.98.1 and the minimal feature profile:

| Test group | Result |
| --- | --- |
| New post-merge routing, scheduling and launch regressions | 15 passed |
| Existing review-role model/route/effort tests | 7 passed |
| Existing automatic-judge parent-target test | 1 passed |
| Existing review-child transcript test | 1 passed |
| Existing advisor/picker tests | 19 passed |
| Existing remote-client regression filter | 212 passed |
| Workflow YAML validation | 5 passed |
| Advisor acceptance-report validation | 9 passed |
| Competitive-evaluator unit tests | 32 passed |
| Module declarations and dependency boundaries | Passed |
| Formatting and staged patch whitespace | Passed |
| No newly oversized Rust files / no growth in touched oversized files | Passed |

The Rust filters overlap: these are 255 successful test executions, not a claim of 255 different tests. The 46 Python tests include report/fixture checks, not a paid-provider or competitive benchmark campaign.

The validation run's overall status is **failure solely because its final publishing step lacked permission to update workflow files**; all compilation, regression and structural-check steps completed successfully. Publication was separated: the exact tested source was pushed without workflow changes, and the workflow tree was handled through the authorized GitHub connector. The tested candidate archive is pinned by SHA-256 `b418b55a0c0665e0cd15681886e67e16682c19017c0d219f16f3f962274a49da`.

Two existing fixtures were aligned with their actual contracts: the role-selection matrix now creates a fresh app per independent launch; the ownership-completion test distinguishes the required single final response from reopening an ownership retry. The ownership fixture failure was reproduced on the unchanged base before adjustment. The new persisted-child fixture gives its otherwise empty session a title, because untitled empty sessions are intentionally not saved.

The complete all-features, all-platforms, lint/size-policy and live terminal/provider matrix is not certified by these targeted checks.

Four independent probes were first compiled against the unchanged audited implementation and required to fail as actual tests: connected feature controls, preservation of the first launch, exact command-token matching, and automatic end-of-turn scheduling. A compilation error or zero matching tests did not count as reproducing a regression.

The patched coverage includes both input paths, idle/busy states, manual and automatic commands, invalid arguments, stale/duplicate completion, system and observer turns, cancellation, bounded queues, model/effort snapshots, split failure during unrelated main work, connection reset, and neighboring prompt/transfer/fork/workspace collisions.

New production/test helpers remain below the repository's size limit. Existing pending-request types, reload restoration, split-response handling and clear-session tests were extracted into cohesive files; oversized touched modules shrink relative to the audited source. No size baseline, evaluator fixture, acceptance threshold, warning allowance, issue gate, or repository security setting was relaxed.

## Important remaining risks

### 1. Make fork provenance an update boundary — P1

`crates/jcode-app-core/src/update.rs` still identifies `1jehuang/jcode` as its update repository, and `src/cli/selfdev.rs` retains the same upstream source URL. Automatic updating is already skipped for non-release builds, explicit opt-out and executables inside a Git checkout, but those checks do not establish fork identity for every distributed binary.

A fork release can therefore be offered an upstream artifact without the custom features. This is a source-confirmed provenance risk, not a reproduced destructive update in this audit. Until fork-aware distribution is implemented, keep automatic updates disabled for custom fork installations using the existing `--no-update` / `JCODE_NO_AUTO_UPDATE` controls.

Implement an embedded distribution identity (repository, channel, commit, artifact provenance), reject cross-distribution updates by default, and verify it in an installed-binary smoke test. Do not mechanically replace upstream URLs: dependency origins and the fork's available release assets need different policies.

### 2. Make cancellation stop work, not only suppress output — P1

`crates/jcode-app-core/src/advisor.rs` spawns reviews and fences publication with `active_review_id`; reviews also have a timeout. That prevents stale results from becoming current results. It is not immediate cancellation of a superseded provider operation.

Give each role invocation a supervised cancellation handle, propagate cancellation into streaming requests and owned child processes, and account for already-incurred cost. Keep latest-only coalescing and bounded concurrency. Acceptance must prove that disable, replacement and session teardown stop resource use—not merely that stale notes disappear.

### 3. Enforce read-only reviewer capabilities — P1

Review/judge startup messages in `commands_review.rs` say not to edit files or run destructive commands. A prompt is useful guidance but not a permission boundary. This patch does not establish runtime-enforced read-only authority for those roles.

Use role-specific tool allowlists and isolated, disposable snapshots/worktrees. Restrict shell/network capabilities according to the task. Supply a trusted verifier with independently captured evidence. Include malicious repository instructions and attempted write/shell escalation in the tests. Keep the advisor's existing tool-free isolation intact; it should not be weakened to make every role share one execution model.

### 4. Finish measurement before making competitive claims — P1

The existing Phase 0–3 audit and current `scripts/competitive_eval` implementation leave material gaps: `max_cost_usd` is descriptive rather than an enforced runner budget, external side-effect containment is not established by the manifest field, and peak RSS remains reported as zero. Live paired comparisons remain distinct from deterministic fixture success.

Measure trusted-verifier task success, repair cycles, total cost including all roles/canceled work, latency distribution, actual peak memory, and containment violations. Pin task sets and compare matched configurations under the same budget. Add new adversarial cases without modifying the frozen baseline to make a score look better. Deterministic fixtures and opt-in real-provider campaigns should have separate reports.

### 5. Finish write-transaction and recovery boundaries — P1/P2

The existing Phase 1 documentation still records sequential publication in legacy multi-file edits after common preflight; an I/O failure can leave partial publication. Strict stale-write rejection is valuable, but it is not all-or-nothing publication.

Extend the staged transaction approach to the remaining legacy write adapters, test failure between every publication step, and define rollback/recovery ownership. Decide explicitly when unattended work must change guard policy from warning to blocking. Do not silently weaken stale-write protection through fuzzy recovery.

### 6. Restore repository-wide quality credibility — P2

The pinned base already exceeds production/test size budgets in multiple modules. This patch reduces its touched oversized modules but is not a repository-wide debt cleanup. The linked-issue workflow also requires a real issue while this fork has Issues disabled; referencing a pull request is not an honest substitute. The owner must resolve that policy/settings conflict deliberately.

Keep a fast integration suite mandatory, then repair the wider build/lint/size/platform matrix in separate bounded changes. A green narrow suite must not be represented as a green whole repository.

## Suggested next development sequence

**Milestone A: dependable orchestration.** Move scheduling into a daemon-owned, bounded lifecycle keyed by session, task generation and role; persist idempotency and route snapshots. Exercise the same behavior through Enter, generic TUI input, CLI, SDK and headless modes. Inject disconnects, quota errors, delayed/duplicate replies and model switches.

**Milestone B: authority and budget enforcement.** Add capability-scoped roles, fork-safe update provenance, cancellation propagation, and hard resource budgets. Reject unavailable explicit routes visibly rather than silently changing the user's chosen model/account/effort.

**Milestone C: verified edit reliability.** Complete legacy multi-file publication/rollback, stale-write enforcement and crash recovery. The completion criterion is verifier-backed repository state, not an assistant's statement that the task is done.

**Milestone D: evidence-driven optimization.** Run paired evaluations and ablations: main-only, main plus workers, conditional advisor, and independent judge. Measure whether each role adds enough verified success to justify its cost and delay. Make additional agents conditional on evidence rather than enabling every role for every task.

The intended outcome is fewer unverified actions and more reliably completed tasks—not the largest number of agents or settings.

## Evidence map

- PRs: 13, 14 and 15 in `LLMpsycho/jcode`.
- Connected execution: `crates/jcode-tui/src/tui/app/remote/{input_dispatch,key_handling,review_controls,review_launch,split_response,server_events,workspace}.rs`.
- Scheduling/role snapshots: `commands_review_auto.rs`, `commands_review.rs`, `commands_review_model.rs` and the related tests under `tests/`.
- Workflow validity: `.github/workflows/{ci,ios-testflight,windows-smoke,post-merge-audit}.yml`, `scripts/test_workflow_yaml.py`.
- Remaining phase boundaries: `docs/plans/omp-overtake/PHASE0_3_AUDIT.md`, `PHASE1_WRITE_GUARD.md`, `PHASE4_ADVISOR.md`.
- Cancellation/provenance/measurement: `crates/jcode-app-core/src/advisor.rs`, `crates/jcode-app-core/src/update.rs`, `src/cli/selfdev.rs`, `scripts/competitive_eval/{run_one,process_metrics}.py` and its schemas.
