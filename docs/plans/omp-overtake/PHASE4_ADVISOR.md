# Phase 4 advisor implementation record

Status: the ongoing investigative-advisor correction is implemented on
`feat/advisor-agent-parity`, based on `master`
`3e747d3722fe6f809500957d7d55350c40a4fec3`. Final-head validation and merge
readiness are tracked in [PR #18](https://github.com/LLMpsycho/jcode/pull/18);
passing results from earlier PRs do not establish acceptance for this branch.
The approved behavioral contract is
[PHASE4_ADVISOR_AGENT_PLAN.md](./PHASE4_ADVISOR_AGENT_PLAN.md).

This extends the foundation and live controls from PRs #6/#7, completion and
model-selection work in PRs #10/#12/#13/#14, and subsequent command fixes.
The previous completed-turn, non-investigative reviewer did not implement the
ongoing peer-advisor behavior. The current runtime observes safe boundaries,
retains a separate conversation, investigates source, and can cause a bounded
primary correction before completion. It was implemented independently from
OMP; no OMP source or prompts were copied. Phases 5–7 remain outside scope.

A deterministic HTTP fixture exercises actual jcode tools, providers, daemon
and socket dispatch. It is not live-model evidence. The optional OpenAI API
probe described below remains a separate gate and does not require users to
have an API key to use their existing subscription login with the advisor.

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

## Choosing models for the other agent roles

Run `/agents`, `/config agents`, or `/config models`, then choose a role. Direct
commands such as `/agents swarm`, `/agents review`, and `/agents judge` skip the
role list. `/config` also points to these controls.

| Role | Selection scope | Control |
| --- | --- | --- |
| Main | Current session | Existing `/model` picker |
| Advisor | Current session; checkpointed across restart/reload | `/advisor` model, then effort |
| Swarm / subagent | Saved default for new workers | `/agents swarm` |
| Review | Saved default for new review tasks | `/agents review` |
| Judge | Saved default for new judge tasks | `/agents judge` |
| Memory | Saved default for subsequent sidecar tasks | `/agents memory` |
| Ambient | Saved default for subsequent ambient tasks | `/agents ambient` |

Role pickers use the same authenticated catalog as `/model`. Choose a model row
with its desired effort, select its connection with the arrow keys, and press
Enter. A plain model row leaves effort to the provider. `inherit` clears the
saved model, route and effort together. A session `/subagent-model` override
still takes precedence over the saved swarm default. Only models actually
present in the catalog can be selected; display examples are not extra models.

The saved connection includes its subscription/OAuth/API method and named
endpoint or OpenRouter provider pin, without credentials. Explicit choices run
on private provider forks and cannot silently fall back to another account or
model. Missing routes, unsupported efforts, and failed child-session restoration
produce errors. Ambient also respects `ambient.allow_api_keys`; memory requires
a provider that can perform tool-free requests. Existing defaults remain usable
when no role override is set.

Typing `/advisor` now previews the list before Enter. Catalog and effort reads
remain responsive while the main turn is busy. A stale or disconnected daemon
produces connection/reload guidance instead of an indefinite loading row. These
client and server changes require an updated binary and a restarted or reloaded
daemon; a newly built client alone cannot update an already-running old daemon.

### If `/advisor` says `Unknown skill`

`/advisor` is a built-in TUI command, not an installable skill. The plain
`jcode repl` interface has no model picker; exit it and launch normal `jcode`.
The REPL now explains that requirement instead of reporting an unknown skill.
Connected TUI submissions from transcript actions use the same advisor dispatch
as Enter, including while a main turn is busy.

If the normal TUI reports the exact unknown-skill error, inspect `/version` in
that running TUI, then `type -a jcode` and `jcode --version` in the shell. A
merged PR or a rebuilt daemon does not replace an already-running old client.
The standard `scripts/install.sh` and release updater still target upstream
`1jehuang/jcode`, so they do not establish that this fork's advisor is installed.

On macOS/Linux, from a clean, current `master` checkout of `LLMpsycho/jcode`, use
the source installer and its explicit launcher:

```bash
git remote get-url origin
git pull --ff-only
scripts/install_release.sh --fast
"$HOME/.local/bin/jcode" --no-update
```

The source installer builds this checkout, installs its binary, and requests a
server reload. Use the newly opened TUI for `/advisor`; `--no-update` avoids the
upstream automatic update check for that launch. If a custom install directory
is configured, use that installer's printed launcher path instead.

## Named advisors

An empty roster keeps the single `default` advisor. Configure up to eight named
advisors globally in Jcode's `config.toml` under `[[advisor.roster]]`. Each entry
has independent model/route/effort, specialization, live history, notes, budget
consumption, status and session controls. Example:

```toml
[advisor]
enabled = true
instructions = "Check the user's requirements and verify material findings."

[[advisor.roster]]
name = "correctness"
instructions = "Inspect runtime behavior and edge cases."

[[advisor.roster]]
name = "verification"
instructions = "Check whether the evidence supports completion."
```

Use the authenticated picker to select each entry's exact model and effort:

```text
/advisor model correctness
/advisor model verification
/advisor status
/advisor inspect verification
/advisor off correctness
/advisor ack <note-id> verification
```

`/advisor model|inherit|status|inspect|on|off [name]` targets one named advisor.
`/advisor ack|dismiss <note-id> [name]` handles its note; without a name, the
server locates that note within the owner's roster. Untargeted status and
inspect cover the roster; untargeted on/off apply across it. Untargeted model
selection uses `default`, or the first configured advisor when no `default`
exists. Status includes review consumption, retained context message count,
suppression count and any failure or budget-exhaustion explanation. It does
not expose the advisor's private reasoning.

A workspace may specialize the roster in `.jcode/advisor.toml`, relative to the
session working directory. Its syntax uses `instructions` and `[[roster]]`
without the global `advisor` prefix:

```toml
instructions = "Follow this project's acceptance criteria."

[[roster]]
name = "verification"
instructions = "Inspect integration coverage for changed behavior."
```

Project entries replace matching global names and append new names; shared
instructions concatenate. A nonempty roster replaces the implicit default.
Names are 1–48 lowercase ASCII letters, digits, hyphens or underscores; duplicate
names and invalid/oversized configuration fail visibly. Project files are
bounded to 64 KiB and cached for one second, and combined specialization is
bounded to 16 KiB per advisor. Project configuration cannot enable advisors
session-wide, expand tools, relax budgets, or widen `allowed_runtime_keys`.
Named entries inherit those constraints. An explicit session choice is durable
metadata; it does not rewrite the global or project roster.

## Runtime behavior and evidence

An enabled primary turn establishes a visible task anchor, standing project
instructions, recent visible session context and an investigative adapter. It
publishes incremental updates at safe model/tool boundaries while the advisor
runs independently. Completed-user-turn cadence is counted separately from
these intermediate observations. A later `continue` retains earlier task
requirements while identifying that later instructions may supersede them.

Each advisor sends its retained conversation back to its private provider:
updates, its own responses, and matched investigative tool results. Ordinary
advisor prose remains private. Successful silence produces no note. The
structured `advise` action carries severity, a stable concern identity,
evidence and recommended action; the legacy structured-note adapter remains
compatible. Final-review findings must cite supplied evidence or actual
independent investigative results.

Captured evidence includes visible user/assistant text, bounded tool requests
and results, patch excerpts, diagnostic deltas, explicit verification process
results, unfinished todos and acceptance criteria. Primary hidden reasoning,
provider signatures, raw images and encrypted compaction items never cross the
primary/advisor observation boundary. Synthetic advisor deliveries are excluded
from subsequent visible deltas to avoid reviewing the advisor's own message.

The Git source supplies a tracked working-tree **patch against HEAD**, with
an 8 KiB read ceiling and 350 ms deadline. It may include earlier edits and
omits untracked files, submodules and recognized credential stores; separate
file-revision metadata identifies actual writes during the current primary
turn. External diff, text conversion and fsmonitor execution are disabled.
Configured clean/process filters make the source unavailable. System/global
Git configuration and inherited command-line overrides are excluded. Missing
Git/HEAD, unavailable sources and truncation are labelled, never reported as a
clean verification result.

Foreground `bash` verification retains actual process completion and exit code.
An ordinary successful command, background job, or completed primary response
is not proof of passing tests. Todo criteria and declared checks remain labelled
requirements/claims. Diagnostics retain bounded range/severity/message data,
not raw language-server responses, with high-severity findings retained.

## Investigation, permissions and provider isolation

The advisor can use `read` and `agentgrep` in its read-only grep/find modes,
when those implementations are granted to the owning primary session. Every
invocation rechecks the grant, operation capability and investigative policy.
Unknown, effectful, batch/delegated, shell and MCP tools are unavailable. Names
alone never authorize an implementation. Reads are workspace-confined after
canonical path resolution; credential stores and symlink escapes are refused.
Search excludes hidden and ignored files and cannot inherit a ripgrep
preprocessor command. Read calls are limited to regular files up to 1 MiB and
200 lines, with bounded redacted outputs.

Investigation uses isolated tool context and does not alter the primary's read
coverage or verification capture. It does not execute arbitrary policy hooks.
If a pre-tool policy hook is configured, investigation is unavailable and the
advisor receives an explicit limitation; the adapter neither bypasses that
policy nor runs its shell commands.

Only provider routes that can suppress autonomous built-in tools are eligible.
The provider fork is explicitly restricted to the supplied tools before review;
unexpected native tool actions or provider-side results fail the review. Grok
ACP and the deprecated Claude CLI runtime remain excluded when their native or
inherited MCP behavior cannot be disabled. The historical `tool-free` capability
check establishes native-tool isolation; it does not mean this advisor receives
an empty explicit tool list.

Model/effort selection uses the same authenticated catalog as `/model`. A session
selection takes precedence over the configured exact route/model. Otherwise
interactive and selfdev-guardian use `reviewer_model`, and final-review uses
`verification_model`; absent a configured role, the primary route is inherited
on a private fork. Exact `allowed_runtime_keys` restrict evidence recipients:
`[]` denies all, omission inherits available authenticated routes. Unknown,
unavailable, denied or ambiguous routes fail visibly. An explicit model never
silently falls back or changes the main model, effort, credential pin or provider
session. Autonomous `swarm` effort choices are not advisor effort options.

In interactive mode, advisor feedback does not gate the primary's corrective
tools. Only explicit **selfdev-guardian** mode gates future effectful operations
on unresolved blocking notes. Batch subcalls and unknown/effectful operations
still pass the central capability check. Existing permissions and write guards
apply independently. Corrupt durable advisor state remains a visible,
fail-closed exception until explicit disable or repair.

## Delivery, suppression and cancellation

During active work, concerns enter the nonurgent safe-boundary queue outside
the interruption cooldown; blockers are urgent and nits remain visible asides.
They are independent advice for the primary to weigh and correct or explain. They do not forcibly cancel an already-started primary tool.
At terminal completion, ordinary concerns/nits remain visible asides and do not
start a restatement turn. An eligible late blocker can continue the same user
invocation before the final socket completion event, with at most three
corrective continuations. The terminal drain has a 60-second deadline; an
incomplete drain or exhausted correction allowance produces a visible notice.

A bounded concern ledger uses stable advisor-supplied identity, with normalized
legacy fallback. Rewording an already-reported concern does not create another
note at the same severity. Acknowledgment/dismissal suppresses that concern for
the configured completed-user-turn window, default two; repeated handling does
not extend it. **Observation continues during immunity**, and unrelated concerns
remain eligible. A separate owner-wide `interrupt_immunity_turns` window,
default three completed user turns after delivered interruption, makes further
concerns non-interrupting while blockers remain eligible. Named advisors share
this interruption cooldown. Higher-severity escalation and duplicate blockers
are tested separately. Acknowledgment/dismissal cancels the current review generation and
pending update so a late response cannot immediately resurrect handled advice.

One update runs per advisor; pending arrivals coalesce into a bounded next
update including intermediate evidence and final state. Explicit cancellation,
disable, model replacement, history reset and session disposal fence stale
publication. Dropping a cancelled primary request also cancels its advisor
transport/tool future. A late response cannot resume a user-stopped run. Named
advisor controls are isolated; owner stop/reset/disposal covers the whole roster.

## Resource bounds and restart semantics

| Resource | Bound / behavior |
| --- | --- |
| Evidence snapshot | 32 KiB serialized; up to 12 primary-tool summaries, bounded fields and explicit truncation |
| Visible observation | 16 KiB recent context; individual visible blocks bounded to 4 KiB |
| Per-update advisor work | Up to 6 model steps and 12 investigative calls; repeated identical investigation rejected after two attempts |
| Deadlines | 60 seconds per review; 5 seconds per investigative call; 60 seconds terminal drain |
| Live conversation | Up to 12 complete exchanges / 192 KiB, additionally fitted to provider context budget |
| Retained anchor | Bounded original objective and initial visible task/project context retained when old exchanges are evicted |
| Review starts | `max_reviews_per_session`, default 100 per advisor; lifetime counter survives restart/history changes |
| Note publication | One accepted structured finding per update, subject to configured note budget and suppression |
| Durable state | Up to 32 bounded note records, a bounded concern ledger, at most 256 KiB per checkpoint |

History maintenance removes whole exchanges, preserving tool-call/result pairing
and the task anchor. It is deterministic bounded retention, not an LLM summary
or full-context promotion system. Truncated/omitted evidence is explicit.
Disabled or zero-budget operation does not collect evidence or call providers.
Budget exhaustion is visible in status; it is not silently replenished by rewind
or restart.

Checkpoints live at `state/advisor` under the configured Jcode home. Filenames
hash session identities; named runtime identities are owner-scoped. Atomic,
owner-only checkpoints retain enable and canonical model/effort overrides,
review/turn counters, mode, bounded redacted notes/dispositions, concern
suppression and interruption-cooldown state. They do not contain credentials, provider handles, raw
investigative output, private reasoning or the full live conversation.

Resume restores controls and budget, starts idle, and reconstructs live context
from current visible primary state at the next explicit user turn. It does not
replay historical interrupts. Rewind/undo/applied compaction invalidate pending
reviews, notes and live history while preserving explicit model/enable controls
and lifetime budget. Corrupt state is not overwritten by automatic review;
failed writes are reported as nondurable controls. Persisted notes and
investigative results are redacted even if request snapshot redaction is disabled.

## Requirement traceability and current validation

Paths without a crate prefix are under `crates/jcode-app-core/src/`.

| Behavior | Implementation | Required regression / acceptance |
| --- | --- | --- |
| Observe active primary work and retain requirements | `agent/advisor_live.rs`, primary loop/turn integration | `agent/advisor_live_tests.rs`: visible-only capture, truncation, `continue` task anchor |
| Read evidence independently | `advisor/investigation.rs`, `tool/advisor.rs` | Real source read not present in the prompt; denied effects/unknown tools/revoked grants; path/credential/symlink/hook/preprocessor restrictions |
| Retain real advisor conversation and healthy silence | `advisor/runtime.rs`, `advisor/history.rs` | `advisor/runtime_tests.rs`: later provider input contains earlier exchanges, paired tool results, no note from silence/private prose |
| Coalesce, suppress and cancel | `advisor.rs`, `advisor/suppression.rs`, `advisor/delivery.rs` | Intermediate/final update retention, handled paraphrase versus unrelated blocker, escalation/deduplication, cancelled future and late delivery |
| Bound terminal correction | `agent/advisor_live.rs` | Headless and streaming late blocker correction without another user prompt; cancel during drain and dropped primary future |
| Redacted patch/diagnostic/verification/todo evidence | `advisor/evidence.rs`, tool producers and stable tool/task DTOs | Real Git patch, bounds, unavailable/filter sources, diagnostic priority, real exit codes, criteria preservation |
| Durable restart controls | `advisor/persistence.rs`, model selection and lifecycle paths | Restart/redaction/bounds/corrupt state/write failure, no historical interrupt replay or restored raw history |
| Named roster and targeted controls | `advisor/roster.rs`, `advisor/roster/project.rs`, config types, server/TUI/client controls | Independent routes/efforts/histories/notes, default migration, duplicate/invalid names, project precedence and permission preservation, restart/disable isolation |
| Explicit provider tools and exact role route | `advisor/routing.rs`, provider contract/runtime implementations | Native-tool restriction and rejection, unchanged main route/effort/session, exact authenticated endpoint and account |
| Complete feedback cycle over transport | `scripts/test_advisor_acceptance.py`, `scripts/test_advisor_agent_acceptance.py` | Real isolated daemon observes, independently reads source, advises, primary actually edits source, one final `done`; healthy silence and in-flight cancel |

Final-head formatting, Rust/provider/TUI/SDK tests and isolated selfdev/socket
acceptance are enforced and recorded by the **Advisor acceptance** workflow in
[PR #18](https://github.com/LLMpsycho/jcode/pull/18). That workflow must pass
before merge readiness; its uploaded fixture report records exact binary and
scenario provenance. Local Unix socket creation/connect is denied with `EPERM`,
so local compilation and unit tests are not claimed as socket acceptance. The
full TypeScript SDK suite has passed in CI; earlier development validation
includes a selfdev build and 103 advisor tests. The acceptance harness starts
its own built binary, home, workspace, runtime directory, socket
and process group with telemetry disabled. It never installs/promotes a binary
or contacts an existing daemon. Deterministic mode uses the real Responses API
transport against a local fixture, including all three operating modes,
model/effort isolation, restart/reload, controls, history reset, the investigative
feedback cycle, silence and cancellation.

```bash
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh test --locked -p jcode-app-core --lib advisor
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh test --locked -p jcode-base --lib advisor
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh test --locked -p jcode-tui --lib advisor
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh test --locked -p jcode-tool-core --lib
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh build --locked --profile selfdev --bin jcode
python3 scripts/test_advisor_acceptance.py --binary target/selfdev/jcode
```

## Historical validation and separate live-model gate

The initial completion run at `8a2128dd062c27f0c6941fa768b5944efeff06bb` passed
32 app-core advisor tests, 3 base tests, 7 tool-contract tests and all three
isolated socket modes. Its binary SHA-256 was
`4e0994efde18f6395871f4c62a374fc44d0cd6908e61e60f7df5a4b16c45f901`.
Formatting correction followed in another commit; final-head results for that
completed-turn design are recorded in PR #10. PR #13 records the subsequent
persistence, provider routing and public SDK/control audit. Those results are
historical provenance, not validation of this investigative runtime.

PR #14 added role model/effort selection; the remote automatic review/judge
scheduler previously deferred there was subsequently wired in PR #16. It is
not an outstanding advisor-parity gap. Prior audit notes also recorded broader
repository gates (duplicate CI `env` mappings, size-budget debt, and the
linked-issue check conflicting with disabled Issues). Recheck their current
status separately; this document does not claim those gates passed.

A real-model run remains separate from deterministic acceptance. Using an
already-configured OpenAI API credential, the optional harness command is:

```bash
python3 scripts/test_advisor_acceptance.py --binary target/selfdev/jcode --live --model gpt-5 --report advisor-live-report.json
```

This legacy OpenAI-specific live probe exercises modes and restart controls;
it does not run the fixture-only investigative/cancellation scenarios or prove
quality parity across subscription providers. The report includes provenance,
requested model, binary SHA-256 and inspected verdicts; it is bounded, redacted
and owner-only, and an unsuccessful run creates no success report.

`.github/workflows/advisor-live-acceptance.yml` also supports manual dispatch
with repository secret `ADVISOR_ACCEPTANCE_OPENAI_API_KEY` and optional variable
`ADVISOR_ACCEPTANCE_MODEL` (default `gpt-5`). Its automatic billable trigger is
restricted to the original completion branch, not this parity branch. Credentials
are supplied only to preflight/live steps, never committed or printed. Record
and inspect successful live verdicts separately before closing the live-model
gate. No live-model success, OMP competitive win or measured resource-regression
floor is claimed here. See [PHASE0_3_AUDIT.md](./PHASE0_3_AUDIT.md) for the earlier
bounded audit of Phases 0–3.
