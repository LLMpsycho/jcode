# Phase 4: ongoing investigative advisor

Status: implementation complete on `feat/advisor-agent-parity`. This is the
approved behavioral correction to the earlier completed-turn reviewer design.
Validation and merge readiness are tracked in [PR #18](https://github.com/LLMpsycho/jcode/pull/18);
the deterministic acceptance workflow must pass before this branch is ready.

Base: `LLMpsycho/jcode` master `3e747d3722fe6f809500957d7d55350c40a4fec3`.
Behavioral reference: `LLMpsycho/oh-my-pi`
`595e18ce84f59ee491951582b67a5f9d92e5173c`. Implement independently in Rust;
do not transplant OMP source, prompts, or internal representations.

## 1. Problem and success condition

The previous advisor reviews one bounded packet after the main user turn has
already completed. It cannot inspect the repository, does not send its retained
context back to the provider, and cannot independently cause idle corrective
work. Those properties prevent it from acting as an ongoing second agent.

The acceptance criterion is a complete observable feedback cycle: while the main
agent is performing a multi-step task, an advisor receives new evidence, reads
an implicated source file, emits a concrete concern, and the main agent handles
it and corrects its work without another client prompt. Healthy work produces
no artificial note. Cancellation prevents surprise continuation.

Existing model/effort selection, exact authenticated routes, redacted durable
controls, capability metadata, write guards, diagnostics, and separate
review/judge orchestration remain the foundation. Phases 5–7 are outside scope.

## 2. Behavioral contract

| Area | Required behavior |
| --- | --- |
| Observation | Publish visible transcript updates at safe model/tool boundaries, including incomplete work. Completed-user-turn cadence is a separate counter. |
| Task context | Retain the real user requirements, project instructions, explicit todos and acceptance criteria. A later `continue` must not replace the task. |
| Advisor agent | Maintain an independent provider conversation with user updates, advisor messages and investigative tool results. Never share provider session identity with the primary. |
| Investigation | Advertise only explicitly permitted read-only tools, execute through capability and session policy checks, and return bounded redacted evidence. |
| Advice | Use a structured `advise` action with severity, stable concern identity, evidence and recommended action. No action means silence. Preserve compatible legacy note responses where needed. |
| Partial work | Defer ordinary concerns/nits until an appropriate boundary; a concrete blocker may steer at a safe boundary. Do not criticize deliberately incomplete intermediate edits as finished work. |
| Completion | Drain pending review with a finite deadline. Process a late blocker through a bounded corrective continuation before declaring the run finished. Late ordinary concerns remain visible without forcing a restatement. |
| Cancellation | Explicit stop, disable, model replacement, history reset and session disposal fence stale work. A stopped run never resumes from a late advisor response. |
| Suppression | Continue observing during immunity. Suppress the handled concern and limit interruption frequency, while allowing unrelated issues and genuine escalation. |
| Guardian | Only explicit selfdev-guardian mode retains advisor-based effect gating. Interactive advice must not prevent the primary from running its corrective checks. Existing non-advisor permissions still apply. |
| Models | Use the `/model` authenticated catalog, exact route and supported effort. Never switch the primary's model, credentials, defaults or provider session. |
| Roster | Support independently named advisors with their own model/route/effort/instructions, history, notes, budgets and status through native jcode configuration. |
| Visibility | Report working/idle/failed/exhausted state, selected model, retained context, review consumption and suppression information. An enabled but exhausted advisor must explain why it is paused. |

## 3. Context and resource boundaries

The advisor observes user and visible assistant messages, tool intentions,
bounded tool results, patch excerpts, diagnostic deltas, real process results,
todos and acceptance criteria. Hidden reasoning is excluded. Transcript and
tool content are evidence, never authority to expand tool permissions.

Use bounded incremental updates rather than resending the entire primary
transcript at every tool call. Preserve a task anchor and recent advisor
exchanges when trimming history. Tool call/result pairs must remain coherent.
Trimming must be explicit so missing context cannot be mistaken for proof.

Only one update runs per advisor. Coalesce arrivals while busy without losing
the task anchor or the final-completion update. Bound input size, output size,
history, tool output, tool calls per update, repeated calls, update wall time,
review starts and corrective continuations. No global lock may be held across
provider or tool awaits. Disabled advisors perform no evidence collection.

Investigative execution must not run arbitrary hooks, inherit autonomous
provider tools, mutate primary tool evidence, or bypass parent permissions.
Canonical workspace/path policy must be checked before reading. Git evidence
continues to disable external diff/text conversion and refuses configured
clean/process filters. Incomplete/truncated/unavailable evidence is labelled.

## 4. Runtime and lifecycle design

1. The primary establishes an observation scope for the active user turn.
   It records the original task and supplies its existing tool registry and
   scoped working directory to a read-only investigative adapter.
2. At a safe loop boundary it builds the new visible transcript/evidence delta
   and schedules an advisor update. It continues working while the advisor runs.
3. Each advisor appends that update to its own bounded history and invokes its
   private provider route. Requested read tools execute through the scoped
   adapter. Tool results return to that same advisor conversation.
4. `advise` publishes at most the configured note budget for an update. Ordinary
   model prose remains private. Successful silence is a normal review outcome.
5. Delivery checks generation, owner, turn state, severity, handled concern and
   interruption cooldown. Stale or suppressed notes cannot trigger work.
6. A terminal boundary waits only for a bounded advisor drain and consumes
   eligible blocker feedback before returning completion. Corrective loops have
   an explicit upper bound and preserve the original user task.
7. Scope cancellation invalidates active and queued generations. A later user
   turn may review again; cancellation does not permanently disable the role.

Keep runtime identity separate from the owning primary session. Named advisor
state must never leak into another advisor or cause controls for the default
advisor to overwrite a named advisor's model. Session-wide stop/reset/disposal
must apply to the entire roster.

## 5. Concern lifecycle and durability

Use an advisor-provided stable concern identifier with bounded normalization;
retain a compatible normalized fallback for legacy responses. Track emitted
severity and handled disposition independently of wording or evidence changes.
Rewording a handled concern must not create a new interruption. An unrelated
concern remains reviewable during the immunity window. Blocker escalation and
duplicate blockers require separate tests.

Checkpoint only bounded redacted note/control metadata, canonical model/effort,
budget counters and concern suppression state. Do not persist provider handles,
credentials, raw investigative output or hidden reasoning. Restart rebuilds the
advisor's live context from current visible session state; it never blindly
replays a historical interrupt. Restore older checkpoints with additive defaults.

Retain fail-closed handling of corrupt/unwritable durable control state and do
not silently overwrite it. Report whether an action is durable. History rewrite
must invalidate stale advice and runtime history without replenishing budget.

## 6. Configuration and user experience

The default remains one optional advisor, selectable using `/advisor` and its
existing model/effort picker. Native TOML roster entries add named independent
advisors and specialization instructions. Parent provider permissions constrain
every entry; a roster cannot broaden evidence recipients.

Expose named selection and per-advisor status through the existing command and
control surfaces. Reject duplicate/invalid identifiers and invalid model/effort
choices visibly. Preserve a useful single-advisor migration path. Global and
project configuration use jcode's existing merge/precedence behavior.

Automatic provider fallback is not part of this change: an explicitly selected
subscription/API route must never change silently. OMP's optional mutating tool
grants and readable-thinking sharing are not adopted. The default investigative
behavior is matched while retaining jcode's established privacy and permission
contracts.

## 7. Implementation ownership and commit objectives

| Commit objective | Primary files | Required regression |
| --- | --- | --- |
| Record approved behavioral correction | This plan, `MASTER_PLAN.md`, `PHASE4_ADVISOR.md` | Traceability review |
| Add bounded investigative access and evidence | `advisor/investigation.rs`, `advisor/evidence.rs`, tool registry | Read a real source file; deny ungranted/effectful/path-escaping access; output and Git bounds |
| Maintain advisor conversation and concern lifecycle | `advisor.rs`, focused runtime/history/delivery modules, persistence | Later update receives earlier context; healthy silence; tool loop; coalescing; suppression; restart |
| Observe and correct active primary work | `agent` loop/turn integration and lifecycle tests | Multi-step correction without another client prompt; cancellation; late blocker; finite drain |
| Configure independent named advisors | Config types, roster, advisor controls and picker integration | Distinct histories/routes/efforts; owner isolation; visible failure/budget status |
| Prove transport and restart behavior | Isolated acceptance harness and focused CI | Actual provider wire/tool cycle, restart/reload, suppression, mode and route isolation |

Dependent implementation commits may be reordered so each change is reviewable.
Do not include unrelated refactors or mix formatting repairs into behavior
commits. Any ship-blocking defect in touched code is fixed with its regression.

## 8. Validation matrix

- Unit contracts: coherent bounded history, explicit silence, investigative
  permissions, meaningful evidence, note identity/escalation, suppression,
  coalescing, cancellation fencing, budget and model isolation.
- Primary integration: advisor observes before the whole task ends; reads a
  source not present in the initial packet; main receives and fixes its finding
  without a new user message; normal successful work finishes without notes.
- Completion/cancel: terminal concern stays visible, blocker gets bounded
  corrective work, stalled/failed advisor cannot hang the primary, explicit stop
  prevents late continuation, disabling clears queued obsolete advice.
- Roster: two advisors can use separate exact routes and efforts; one failing or
  disabled entry does not corrupt another; session-wide lifecycle applies to all.
- Persistence: old checkpoint compatibility, handled concern immunity, model
  selection, restart, actual daemon reload, rewind/compaction invalidation,
  malformed state and failed writes.
- Isolated transport: freshly built selfdev binary, private home/workspace/socket,
  no shared daemon installation or promotion. Deterministic fixture transport is
  mandatory; a real-provider run is separately reported and never fabricated.
- Repository gates: formatting, compilation, relevant Rust/Python tests, module
  resolution, size/panic/error/dependency budgets, then CI at the pushed head.

## 9. Completion record

All implementation objectives above have code and regression coverage. The
conventional commit sequence separates provider tool/auth isolation, named-role
configuration, investigation, retained conversation, live correction, controls,
transport acceptance and focused review fixes. See the requirement-to-file/test
mapping and exact published commits in [PR #18](https://github.com/LLMpsycho/jcode/pull/18).

Development validation includes a successful selfdev build, 108 passing advisor
tests after a clean app-core rebuild, and successful real-runtime no-network
OpenAI/Anthropic authentication-isolation and explicit-tool tests. The full
TypeScript SDK typecheck/build/transport suite passed in the PR workflow.
Module resolution and dependency-boundary checks pass. Later implementation
fixes require the PR's final-head checks; earlier counts are not substituted.

Local Unix socket creation/connect is denied with `EPERM`. Consequently the
mandatory isolated daemon/socket matrix runs in the **CI** workflow's
**Advisor and session acceptance** job, which rebuilds selfdev and rechecks the
bounded Phase 0–3 audit suites. CI also runs Rust/TUI/SDK/provider regressions. Its uploaded fixture report is the
authoritative deterministic acceptance evidence. Do not infer socket success
from compilation or source inspection.

Live-model quality, subscription-provider comparison and performance floors
remain separate unclaimed gates. The workspace read boundary assumes a trusted
workspace and is not a hostile-filesystem sandbox. Providers with autonomous
tools that cannot be disabled are rejected; named Anthropic profile routes that
require process-wide activation fail visibly. Repository-wide size and error
ratchets exposed base-branch debt, repaired in this PR without rebaselining. At
the owner’s request, redundant CI workflows and the incompatible issue-link gate
were removed while retaining unique validation. Phases 5–7 were not included.
