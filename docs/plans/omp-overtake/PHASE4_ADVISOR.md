# Phase 4 advisor implementation record

Status on 2026-09-04: the remaining implementation is delivered on the Phase 4
completion branch, extending the foundation and live controls from PRs #6/#7.
Deterministic unit and isolated selfdev/socket acceptance have passed. The
live-model acceptance gate remains open: this execution environment has no
provider credential. A local HTTP fixture is not claimed as live-model evidence.
Final-head CI results and any outstanding gates are recorded in PR #10.

## Requirement traceability

| Remaining requirement | Implementation | Verification |
| --- | --- | --- |
| Bounded diff summaries | `advisor/evidence.rs`: tracked working-tree numstat against HEAD, 8 KiB read ceiling, 350 ms deadline, no external diff/text conversion; per-turn revision metadata identifies actual tool writes separately. | `bounded_diff_reads_actual_changes_and_reports_unavailable_sources` exercises a real temporary Git repository and unavailable sources. |
| Diagnostics and verification results | `advisor/evidence.rs`, `tool/mod.rs`, `tool/lsp.rs`, `tool/lsp/diagnostic_output.rs`, `tool/bash.rs`, `jcode-tool-types`: capture structured producer metadata after post-edit diagnostics; explicit foreground exit codes; diagnostic deduplication; no raw terminal output. | Evidence scoping/redaction/failed-check tests; highest-severity diagnostic retention test; isolated provider input assertions. |
| Todos and acceptance criteria | `jcode-task-types::TodoPlan`, `tool/todo.rs`, `advisor/evidence.rs`, TUI plan rendering: explicit bounded `acceptance_criteria`, current unfinished todos, user objective, and declared plan/check state. | `tool/todo_advisor_tests.rs`: retain omitted criteria, clear explicit empty criteria, report changes, reject over-limit input, load legacy plans. |
| Restart-safe minimal controls | `advisor/persistence.rs`, agent construction/restore/reset paths, server controls: atomic owner-only checkpoints, opaque note IDs, inspect/acknowledge/dismiss/enable/disable and lifetime budget restoration. | Persistence tests cover restart, redaction, bounds, corrupt files, and write failures; socket acceptance kills and restarts the daemon after successful controls. |
| Handled-concern immunity | `advisor.rs`: default two completed turns without review after a note is first acknowledged/dismissed; window survives restart; repeated handling does not extend it. | `handled_note_immunity_survives_restart_and_prevents_paraphrase_storms`; socket acceptance observes no review calls for two turns, then exactly one review. |
| Capability-based enforcement | `jcode-tool-core::ToolCapability`, builtin implementations, central registry check after the awaited pre-tool hook and immediately before execution. | Arbitrarily named unknown tool is denied, renamed reader remains usable, nested batch cannot create its target file, handled/disabled blockers release future calls. |
| Permission-aware role routing | `advisor/routing.rs`, typed/default config: reviewer and verification role requests, canonical route selection and exact authenticated-runtime allowlist on a private provider fork. | Role selection, unchanged primary, denied/unavailable/unknown/ambiguous routes, positive exact credential-route permission tests. |
| Evidence-grounded mode behavior | Strict structured JSON, tool-less mode-specific contracts, final-review evidence entries checked against supplied evidence; provider error events and incomplete streams fail without publishing a note. | Mode-capture tests, invented-evidence rejection, malformed/error/EOF tests, all three modes through the production OpenAI HTTP transport. Live-model command below remains unrun. |

Paths without a crate prefix in this document are under
`crates/jcode-app-core/src/`; stable DTOs remain in their leaf crates. No OMP
implementation code was copied.

## Evidence contract and limits

Inputs contain the objective, latest completed primary response, at most 12
current-turn tool summaries with declared intent, diff, new diagnostics,
verification state, unfinished todos, and acceptance criteria. Capture resets at
the start of streaming, captured, and ordinary turns. Historical tool summaries
are not reused as evidence for a new turn. Serialized advisor input is limited to
32 KiB; provider output, individual fields, and retained private context are also
bounded. Disabled or zero-budget operation does not collect evidence or call the
advisor provider.

The Git summary explicitly identifies its scope: tracked working-tree changes
against HEAD can include earlier work and omit untracked files. Missing Git,
missing HEAD, absent diagnostics, timed-out sources, and unavailable todo state
are reported as unavailable, never as a clean verification result. File-revision
metadata independently records writes during the reviewed turn.

A foreground `bash` call may declare `verification: true`; the advisor receives
its actual process completion and exit code. A successful ordinary command or
primary turn is not proof that tests passed. An unfinished/background check is
reported without completion evidence. Todo criteria and feedback-loop claims are
labelled agent-declared requirements/checks, not independent outcomes.

Recognized secrets are redacted before retention and transmission when configured
with `redact = true`. Persisted notes are always redacted, including when request
redaction is disabled. Inline key assignments and recognized OpenAI token formats
are covered. Only bounded diagnostic range/severity/message metadata is exposed
for direct diagnostic reads; raw language-server data is excluded. Highest
severity findings survive text truncation.

## Restart, history and control semantics

- Checkpoints live under the configured Jcode home at `state/advisor`, independently
  of the ephemeral runtime/socket directory. File names hash session identifiers.
- Persisted state is limited to version, enable override, turn/cost counters,
  immunity counters, and at most 32 bounded note records. Individual records are
  capped at 4 KiB after JSON escaping; files are capped at 256 KiB.
- Resume restores controls, handled dispositions, unresolved blockers, and review
  budget. It starts idle with empty private context and no pending request or
  replayed interrupt. Closing a live session drops in-memory state, not its
  restart checkpoint.
- Rewind, rewind undo, and applied compaction invalidate notes, pending reviews,
  private context, deduplication, and queued advisor notifications. They preserve
  explicit enable/disable, lifetime cost counters, and the handled-note window.
- Acknowledging/dismissing cancels any current review generation and pending review
  so late paraphrases cannot immediately resurrect a handled concern. Other
  unresolved blockers continue to gate effects. Disabling fences publication and
  releases future calls without cancelling a tool that already started.
- Corrupt or unwritable checkpoints are visible through status/control results.
  Corrupt state fails closed for effects until explicit disable or repair; it is
  never automatically overwritten by a scheduled review. Failed control writes
  are not reported as durable success.
- Reviews are bounded by cadence, per-session starts, one active request, one latest
  pending request, a timeout, and per-turn note limits. A full set of retained
  unresolved blockers does not produce uninspectable extra notes.

The existing `/advisor status|inspect|ack <id>|dismiss <id>|on|off` TUI and public
client/socket controls remain the product surface. No redundant standalone CLI
control command is added.

## Enforcement and routing

Every registered tool declares a capability through the tool interface. Unknown
implementations/actions are conservative; names have no authority. Read-only
operations remain usable; effects require advisor clearance. Batch delegates to
the same registry for every subcall. DAP evaluation is effectful. LSP mutation,
reload and unknown actions are effectful, while explicit read/preview actions
remain available.

`model` is an explicit advisor override. Otherwise interactive and
selfdev-guardian use `reviewer_model`, and final-review uses `verification_model`.
Absent a role request, the existing primary route is inherited without mutation.
`allowed_runtime_keys` limits evidence recipients to exact stable runtime keys;
`[]` denies all, omission inherits authenticated route availability. For example,
`openai-api:gpt-5` selects the API-key route and `openai-api-key` is its permission
key. Unavailable, denied or ambiguous requests fail visibly without fallback.
Only the provider fork receives a structured route selection; primary model,
credential pin, and cached system prefix are unchanged.

## Validation and outstanding gate

The acceptance harness always starts a separately built binary with an isolated
home, workspace, runtime directory, socket, process group, and telemetry disabled.
It never installs/promotes a binary or contacts an existing user daemon. Default
mode drives the real provider transport against a local Responses API fixture.
It verifies bounded/redacted structured input, normal primary tools, tool-less
advisor calls, inspected notes, acknowledge/dismiss, two-turn immunity, process
restart, actual reload/disconnect/reconnect, persisted disable, rewind, and undo
for all three modes. Applied compaction and stale in-flight completions have
focused runtime tests.

```bash
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh test --locked -p jcode-app-core --lib advisor
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh test --locked -p jcode-base --lib advisor
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh test --locked -p jcode-tool-core --lib
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh build --locked --profile selfdev --bin jcode
python3 scripts/test_advisor_acceptance.py --binary target/selfdev/jcode
```

The initial successful deterministic run used commit
`8a2128dd062c27f0c6941fa768b5944efeff06bb`: 32 app-core advisor tests, 3 base tests,
7 tool-contract tests, and all three isolated socket modes passed. Its selfdev
binary SHA-256 was
`4e0994efde18f6395871f4c62a374fc44d0cd6908e61e60f7df5a4b16c45f901`.
The run's formatting gate failed; the generated correction was applied in a
subsequent commit. Follow-up stream/permission/diagnostic tests and final-head
formatting are checked in the PR's focused workflow.

**Still required before declaring the entire Phase 4 acceptance gate closed:**
run the opt-in live-model probe using an already configured OpenAI API credential,
record its model and results, and review each independent verdict. It exercises
all three modes and restart controls without giving the advisor tools:

```bash
python3 scripts/test_advisor_acceptance.py --binary target/selfdev/jcode --live --model gpt-5
```

This invokes billable provider calls. The key is read only from the caller's
existing environment and is not printed or copied into the acceptance report.
No live-model success, competitive OMP win, or resource-regression floor is
claimed by the fixture results. See [PHASE0_3_AUDIT.md](./PHASE0_3_AUDIT.md) for the
bounded audit of preceding phases and repository-level gates outside this slice.
