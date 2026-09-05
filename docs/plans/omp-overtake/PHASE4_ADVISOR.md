# Phase 4 advisor implementation record

Status on 2026-09-04: the remaining implementation is delivered on the Phase 4
completion branch, extending the foundation and live controls from PRs #6/#7.
Deterministic unit and isolated selfdev/socket acceptance have passed. The
live-model acceptance gate remains open: this execution environment has no
provider credential. A local HTTP fixture is not claimed as live-model evidence.
Final-head CI results and any outstanding gates are recorded in PR #10.

Follow-up on 2026-09-05: PRs #10 and #12 are merged. The advisor now has a session model
and effort picker backed by the authenticated model catalog used by `/model`.
The optional OpenAI API acceptance probe below is one test harness; it is not
a requirement for using the advisor with an existing Jcode subscription login.

## Choosing the advisor model

1. Run `/advisor` (or `/advisor model`) to list available, permitted routes from
   your signed-in providers. The provider and authentication method identify
   each route, including when a model has both subscription and API routes.
2. Choose a model, then choose its supported reasoning effort. Models without
   effort controls show a confirmation row. Confirming enables the advisor for
   this session. The main model, main effort, and global defaults stay separate.
3. Run `/advisor inherit` to enable the advisor using the current main model and
   effort. Existing status, inspect, acknowledge, dismiss, on and off controls
   remain available.

Selections are session overrides of configured advisor/reviewer/verification
models. Only canonical route identity and effort are checkpointed, never
credentials or provider context. Restart, reload, rewind and compaction retain
the choice. Availability and `allowed_runtime_keys` are enforced when selecting
and when running a review; a denied or unavailable route never falls back to
another credential path. Selecting a new model invalidates pending reviews so
an old completion cannot publish as the newly chosen advisor.

Picker and wire regressions cover OAuth selection without an API-key route,
effort selection, primary/default isolation, cancellation and late responses.
Provider regressions cover OAuth/API effort inheritance and independent
Copilot, Gemini and Antigravity forks. Socket acceptance checks the selected
model and effort in real transport requests and after restart/reload.

## Post-merge advisor audit

The follow-up starts from merged `master` at
`7c25bb8e25ae1459f75a8358bf1764a61a722051`; this fork has no `main` branch.
The audit closes these additional integration gaps:

| Finding | Correction | Focused regression |
| --- | --- | --- |
| History resets and queued reviews could clear a checkpoint failure without explicit recovery. | Preserve failed durable state across rewind/compaction and refuse automatic pending reviews until recovery. | Corrupt-file preservation, repeated reset, explicit disable recovery, and queued-review failure tests in `advisor/persistence.rs` and `advisor/tests.rs`. |
| Failed legacy controls looked like successful SDK replies. | Return a redacted typed `error` for failed persistence and missing note IDs. | Enable/disable/ack/dismiss failure and redaction tests in `server/advisor_control_tests.rs`. |
| Explicit routes could reuse the currently active compatible endpoint, and subscription forks lost effort. | Select the exact authenticated runtime and preserve managed-subscription effort on the private fork. The managed wrapper also uses its canonical subscription route during construction, model switches and auth refresh, without requiring OpenRouter credentials. | Endpoint/pin/credential-failure, Gemini/compatible route, and effort regressions in `jcode-base` provider tests. |
| A provider with autonomous built-in tools could receive an advisor review. | Require first-class provider support for tool-free requests before preview, selection or execution. | Internal-tool provider rejection with zero review calls; wrapper capability delegation tests. |
| Advisor quota failover could switch a process-wide primary account. | Execute reviews on the selected runtime without account/provider failover. | Quota-failure route isolation and advisor execution tests. |
| Stale picker errors, generic server errors and disconnects could leave loading state or disturb a main turn. | Correlate advisor replies independently, discard cancelled picker results, and clear transient state on disconnect without replaying controls. | TUI cancellation, supersession, generic error, active-turn and disconnect tests. |
| Modern harness API and Rust/TypeScript SDKs lacked advisor controls. | Expose typed controls/results and canonical catalog selections through the existing session bridge. | Pure DTO compatibility, bridge session/request correlation, capability ledger, SDK parity and transport tests. |

`model_options.available_selections` supplies canonical routes that SDK callers
can forward unchanged when requesting efforts or selecting a model. The field is
additive and defaults to empty for older daemons. It contains no credentials.
The picker explains an empty catalog and distinguishes model loading from effort
loading. A runtime that cannot disable built-in tools is rejected with guidance
to choose another advisor model.

The focused acceptance workflow now covers the public API and both SDKs as well
as the existing isolated socket restart/reload/mode checks. The two inherited
formatting failures and two unused test imports are corrected. The unrelated
repository-wide CI definition still has duplicate `env` mappings, size budgets
remain over their existing baselines, and the linked-issue check still conflicts
with this fork's disabled Issues setting. These broader repository gates are not
reported as passed. The live-model gate below remains separate from deterministic
fixture acceptance.

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
are reported as unavailable, never as a clean verification result. Configured
Repository Git clean/process filters also make the diff source unavailable: a plain numstat
can otherwise execute them despite disabling external diff/text conversion.
System/global Git configuration and inherited command-line config overrides are
excluded from both the filter check and diff command. File-revision metadata
independently records writes during the reviewed turn.

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
- Persisted state is limited to version, enable and model/effort overrides, turn/cost counters,
  immunity counters, and at most 32 bounded note records. Individual records are
  capped at 4 KiB after JSON escaping; files are capped at 256 KiB.
- Resume restores controls, handled dispositions, unresolved blockers, and review
  budget. It starts idle with empty private context and no pending request or
  replayed interrupt. Closing a live session drops in-memory state, not its
  restart checkpoint.
- Rewind, rewind undo, and applied compaction invalidate notes, pending reviews,
  private context, deduplication, and queued advisor notifications. They preserve
  explicit enable/disable, model/effort selection, lifetime cost counters, and the handled-note window.
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

The `/advisor` picker and `/advisor inherit|status|inspect|ack <id>|dismiss <id>|on|off`
TUI and public client/socket controls remain the product surface. No redundant
standalone CLI control command is added.

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
focused runtime tests. The server also waits for owner bookkeeping before
forwarding terminal completion on the ordered stream, so immediate follow-up
turns cannot race a stale busy flag. This is covered by readiness/stale-owner
tests and the existing late-attachment stream-order regression.

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

The harness also accepts `--report <path>` to retain a bounded, owner-only JSON
report with the requested model, binary SHA-256, timestamp, inspected independent
verdict for each mode, and explicit restart/reload/immunity coverage flags. It
redacts the caller's exact credential and recognized OpenAI tokens before writing
or printing. No success report is created by an unsuccessful run. Five Python
regressions cover report bounds, redaction, file permissions, missing-credential
preflight, provenance and failure behavior.

### Completing the live gate through GitHub Actions

`.github/workflows/advisor-live-acceptance.yml` runs for this owned completion PR
and fails before checkout/build when the dedicated credential is absent. It never
falls back to the local provider fixture. To finish the pending gate:

1. Add repository Actions secret `ADVISOR_ACCEPTANCE_OPENAI_API_KEY` using an
   existing OpenAI API credential. Do not put the key in a PR, commit or chat.
2. Optionally set repository Actions variable `ADVISOR_ACCEPTANCE_MODEL`; the
   default is `gpt-5`.
3. Rerun the failed **Advisor live acceptance** job. It builds its own selfdev
   binary and runs all three modes with isolated state and read tools available
   to the primary agent; the advisor stays tool-less.
4. Inspect the seven-day `advisor-live-report` artifact, review each verdict, and
   record the successful run and model in PR #10 before closing this gate.

The credential is scoped to the preflight and live test steps. It is absent from
the build and artifact steps. Automatic billable execution is restricted to this
owned PR; manual dispatch is available after the workflow reaches the default
branch. The separate deterministic workflow saves `advisor-fixture-report`,
explicitly labelled `local-http-fixture`.

This invokes billable provider calls. The key is read only from the caller's
existing environment and is not printed or copied into the acceptance report.
No live-model success, competitive OMP win, or resource-regression floor is
claimed by the fixture results. See [PHASE0_3_AUDIT.md](./PHASE0_3_AUDIT.md) for the
bounded audit of preceding phases and repository-level gates outside this slice.
