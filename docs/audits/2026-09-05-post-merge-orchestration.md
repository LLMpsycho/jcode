# Jcode post-merge audit and fixes

Date: 5 September 2026  
Repository: `LLMpsycho/jcode`  
Audited base: `97107d565165c6ad5ef5ea2d7899a03c44dd1c16` (`master`, after PR #15)  
Pull request: [#16 — remote review orchestration and child-session fixes](https://github.com/LLMpsycho/jcode/pull/16)  
Branch: `fix/remote-review-orchestration`  
Verified runtime revision: `797165f040144d0ef134a1bb2591fe32aff2a150`

## Executive assessment

The recent advisor and independently selectable agent-model features exposed gaps between command handling, asynchronous child creation, session persistence, and execution. The highest-value work was closing those integration gaps, not adding another agent role.

PR #16 fixes confirmed problems in automatic review/judge scheduling, connected command routing, overlapping split requests, reconnect recovery, fork/transfer settings, child-session durability, and CI setup. It includes regression tests and remains unmerged. This is a bounded audit, not a claim that the entire repository is bug-free or that every CI check is green.

The latest runtime changes were reproduced against unchanged behavior, fixed, tested in GitHub Actions, and pushed only after the targeted checks succeeded. The final cleanup retains a read-only regression workflow and removes the temporary workflow used to publish that verified patch.

## What was fixed

| Area | Problem and user impact | Change |
|---|---|---|
| Connected commands | Review/judge commands did not share a reliable dispatch path across normal Enter and generic submissions. A built-in command could fall through to skill handling. | Centralized connected dispatch for `/review`, `/judge`, `/autoreview`, and `/autojudge`, without consuming unrelated skill names. |
| Automatic feedback | Remote completions set automatic-review/judge flags but did not consume them. Enabling the feature could do nothing. | Added bounded, serialized automatic scheduling after eligible successful completions. Failed, interrupted, stale, observer, and system-only cases are excluded. The scheduler suppresses duplicate pending jobs and clears connection/session-scoped work at lifecycle boundaries. |
| Concurrent child creation | Review, prompt, workspace, fork, and transfer paths shared pending split metadata without consistent ownership. A second request could replace the first request's prompt or intent. | Added a common split-in-flight guard and explicit ownership. The first request retains its metadata; conflicting submissions are rejected or restored rather than silently replacing it. |
| Split replies and reconnect | Split responses and errors could interfere with main-turn state or be accepted after the originating operation was obsolete. Replaying an uncertain launch after disconnect could duplicate it. | Added split-specific request correlation, stale/mismatched-response handling, matching-error cleanup, and conservative reconnect recovery without automatic replay of uncertain launches. |
| Empty child sessions | Saving an untitled child containing only initial hidden context could return success without creating a session file. The next client then received an ID it could not load. | Explicit children are now persisted before their first visible message. Untouched root panels remain ephemeral. Children are not artificially bookmarked or assigned placeholder titles. |
| Model settings across handoffs | Local prompt forks, remote splits, and transfers omitted some combination of authenticated route, reasoning effort, or explicit role selection. A child could lose the intended settings. | Copy the relevant route/effort/role metadata in each creation path. Remote splits also retain provider, subagent, and automatic-feedback settings. Each child still gets an independent provider conversation; modifying it does not mutate its parent. |
| CI and guardrails | Duplicate top-level `env` mappings made three workflows invalid. General CI depended on obsolete SSH/deploy-key checkout setup. Budget checks assumed branch names that did not fit the fork. | Merged environment mappings without dropping values, removed obsolete SSH checkout setup, disabled persisted checkout credentials, and made the guardrail base-ref resolver support the actual repository branches. Existing budgets were not relaxed. |

Relevant implementation areas are `crates/jcode-tui/src/tui/app/commands_review*.rs`, `app/remote/`, `crates/jcode-app-core/src/server/client_actions.rs`, and `crates/jcode-base/src/session/persistence.rs`. Dedicated tests accompany the changed state transitions and persistence paths.

## Validation and evidence

### Latest fork/persistence follow-up

[Run 33981520374](https://github.com/LLMpsycho/jcode/actions/runs/33981520374) completed successfully. It tested the generated patch and published only the eight explicitly selected source/test files after all validation steps passed. The resulting source commit is `797165f040144d0ef134a1bb2591fe32aff2a150`.

The before-fix run produced **one passing and three failing local-fork tests**. Two failures showed lost route settings; the third showed that the empty child had no persisted file. These were runtime assertion/load failures, not an intentionally broken compilation.

The retained after-fix logs show:

| Suite | Result |
|---|---:|
| Base session suite, including the new persistence regression | 283 passed, 0 failed, 1 ignored |
| New remote split/transfer persistence and route tests | 2 passed, 0 failed |
| TUI post-merge review/fork regressions | 19 passed, 0 failed |

Each of the two server tests exercises explicit versus inherited role selection and empty versus visible history. These are multiple scenarios within two tests, not eight separate test results. The TUI group includes four new local-fork tests.

The same successful job also ran the existing split-session, review-role, remote TUI, and advisor TUI filters, plus formatting, module-file checks, and dependency boundaries. Counts above are per-suite executions, not an invented repository-wide coverage percentage or a deduplicated grand total.

The artifact `prompt-fork-regression-evidence` contains the before/after logs and the exact eight-file source patch. Downloaded ZIP SHA-256:

`b4620fe8449eeac2ff2c51ba0de8660f8a9b28f7c7e2aff0104b1f2decc9f5b210`

Earlier follow-up attempts exposed an empty-session fixture assumption and a return-type error in a new test. They stopped before publishing runtime changes. The successful run supersedes them; the failed attempts are not presented as passing validation.

### Earlier orchestration work

The original audit artifact recorded passing runs of the post-merge review, remote TUI, review-role, advisor, spawned-review preparation, and queued-autojudge filters. The early source-publication run [33979300344](https://github.com/LLMpsycho/jcode/actions/runs/33979300344) passed its functional checks but failed its final publication step. It was not an overall green workflow run. The subsequent focused PR audit [33979823031](https://github.com/LLMpsycho/jcode/actions/runs/33979823031) passed on the earlier PR revision.

The permanent `.github/workflows/post-merge-audit.yml` is read-only and covers workflow YAML, session persistence, server split/transfer regressions, review scheduling and routing, remote TUI behavior, advisor behavior, formatting, module resolution, and dependency boundaries. It is deliberately separate from the broader repository gates.

### Remaining validation limits

The repository is **not globally green**. After repairing workflow loading, the broader CI run [33979822984](https://github.com/LLMpsycho/jcode/actions/runs/33979822984) reached strict all-feature Clippy and reported 39 errors in app-core test code. Existing size/quality-ratchet debt also remains. Those checks were not disabled or rebaselined to manufacture a pass.

The linked-issue gate remains incompatible with the fork's disabled Issues setting. No issue policy, repository setting, or branch protection was changed.

This work does not establish live paid-provider correctness, end-to-end performance superiority, exhaustive platform coverage, or successful real terminal launch behavior. It does establish the tested command/state-machine and persistence behavior. No live paid-provider acceptance was performed.

Direct terminal `git clone` failed because the workspace could not resolve GitHub. GitHub Actions performed the actual checkout; its tracked-source archive was retrieved for local inspection. Rust tests ran in Actions because this workspace has no Rust toolchain.

## Remaining findings and improvements, in priority order

### 1. Enforce review permissions at the tool boundary

Review/judge prompts describe analysis-only behavior, but a prompt is not a capability boundary. The advisor already has a distinct tool-free contract; that does not automatically make ordinary review/judge sessions read-only.

Introduce explicit role capabilities, checked at actual execution and again for nested/batched calls and provider-hosted tools. Deny writes, process spawning, and arbitrary shell execution by default for analysis-only roles. Tests should show that even an adversarial review request cannot mutate a file. Any permitted verification command needs its own constrained execution policy; a label such as “read-only bash” is insufficient.

### 2. Separate fork distribution from upstream updates

Update and self-development URLs still include `1jehuang/jcode`. That is a source-confirmed distribution risk, not a reproduced destructive update. Existing non-release/opt-out/inside-repository guards matter and should not be ignored when assessing the risk.

Add explicit build provenance and an update origin/channel that cannot silently cross from this fork to upstream. Distinguish fork builds in client and daemon diagnostics, verify artifact provenance, and make channel switching an explicit operation. Until a fork release policy exists, retain the documented `--no-update` or `JCODE_NO_AUTO_UPDATE` protection where applicable.

### 3. Move automatic jobs to a durable daemon queue

The repaired TUI scheduler is bounded and reconnect-conservative, but it is not a durable multi-client scheduler. A daemon-owned job ledger would make guarantees survive TUI detachment and enable CLI/SDK clients to observe the same work.

Use a persisted identity such as `(parent_session_id, completion_id, role)` with atomic claim/state transitions and a frozen route/configuration snapshot. Test two attached clients, reconnect during launch, restart before acknowledgement, and a configuration edit while queued. Prefer at-least-once delivery with idempotent effects over an unsupported “exactly once” claim.

### 4. Cancel obsolete provider work and reserve budgets before dispatch

Advisor generation fencing prevents stale results from publishing; it does not necessarily stop an already-running provider future before its timeout. Add cancellation propagation for disable, rewind, supersession, and session close, while preserving publication fences as a second defense.

Pair cancellation with per-session and per-role concurrency, token, and cost reservations. Reconcile estimated and actual charges and expose clear budget-stop reasons. A configuration field or post-hoc cost report is not a hard execution budget.

### 5. Consolidate fork/transfer settings into one execution snapshot

The repaired paths previously copied model-related fields individually. That pattern makes future omissions likely whenever a new provider or role option is added.

Define a typed execution snapshot containing model, authenticated route, reasoning effort, role pinning, permission policy, and budget policy. Make fork, transfer, restore, and worker launch consume it through a shared constructor with explicit inheritance rules. Keep provider conversation IDs and mutable runtime handles outside the inherited snapshot.

### 6. Make editing transactions and evaluation evidence trustworthy

Legacy multi-file publication can still fail after earlier files have been changed. Add preflight validation, staged writes, conflict rechecks, and a rollback journal before claiming all-or-nothing behavior.

For competitive evaluation, enforce cost and time ceilings, contain side effects outside the workspace, report measured rather than placeholder resource metrics, and distinguish deterministic fixtures from live-model evidence. Compare the same tasks and budgets across main-only, advisor, reviewer, and judge configurations. Success rate, regression rate, wall time, cost, and human intervention are more useful than the number of agents involved.

## Practical sequence

Review and merge the bounded fixes only after inspecting the remaining CI failures. Then prioritize execution permissions and fork update provenance, followed by durable scheduling/cancellation/budgets. Transactional editing and measured evaluations should follow before adding broader autonomous authority or more orchestration features.

The PR remains open and unmerged. No force push, paid provider call, release deployment, or repository-policy bypass was performed.
