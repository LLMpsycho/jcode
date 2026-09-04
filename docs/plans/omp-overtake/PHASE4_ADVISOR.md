# Phase 4 advisor implementation record

This file records delivered behavior and remaining work for Phase 4 of
[`MASTER_PLAN.md`](./MASTER_PLAN.md). It deliberately distinguishes the working
foundation from the full advisor product.

## Delivered: advisor foundation

### Configuration and routing

- `[advisor]` is typed and documented in the generated configuration.
- The feature is disabled by default and schedules no provider call or runtime
  state while disabled.
- `interactive`, `selfdev-guardian`, and `final-review` parse as explicit modes.
- Advisor completion uses a fork of the primary provider. An optional canonical
  `/model` selection request is applied only to that fork, preserving the
  primary provider state and cached prefix.

### Runtime and evidence boundaries

- `AdvisorManager` owns session-scoped runtime state, a monotonic turn cursor,
  bounded private context, status, note counters, deduplication state, and a
  bounded last error.
- Streaming, captured, and normal non-streaming turns schedule review after the
  primary turn completes.
- Inputs have explicit fields for objective, completed primary response, concise
  tool results, diff, diagnostics, verification, todos, and acceptance criteria.
- The lifecycle integration populates objective, completed response, concise
  tool summaries with their declared intent, and turn success. The richer diff,
  diagnostics, todo, and acceptance enrichers remain pending.
- Individual fields, tool counts, total serialized input, response size,
  evidence count, and retained private turns are bounded.
- Recognized secrets are redacted before evidence is retained or sent when
  `redact = true`.

### Structured output and delivery

- The advisor must return one strict JSON note with severity, summary, evidence,
  recommended action, and blocking metadata.
- Jcode derives blocking from the configured severity threshold instead of
  trusting the model-provided boolean.
- Duplicate notes are suppressed and note publication obeys the per-turn budget.
- One active review may retain one latest pending review. New completions
  coalesce into that bounded slot instead of creating an unbounded queue or
  silently losing the latest turn, and every provider review has a timeout.
- Configurable cadence reviews one out of every N completed turns, while a
  per-session cap bounds the total provider reviews started by one runtime.
  A zero cap disables scheduling without allocating runtime state.
- Notes use the existing soft-interrupt queue. Completed background notes are
  injected before the next user objective reaches the provider, while existing
  mid-turn safe points handle notes that finish during a turn.
- Advisor parsing, routing, request, and stream failures only mark the advisor
  runtime failed. They do not fail or corrupt the primary turn.
- Closing, clearing, or restoring the owner session removes the prior advisor
  runtime. Unique review generations fence late completions from a recreated
  session with the same public session identifier.
- Rewind, rewind undo, and applied compaction discard the prior advisor cursor,
  private context, deduplication state, and pending reviews so changed history
  cannot receive stale notes.
- Provider and parser failures are redacted before logging or runtime storage.
- Published notes also retain bounded, redacted metadata under opaque `adv-*`
  identifiers. At most 32 note records are held per live session runtime.

### Controls and enforcement

- The protocol supports advisor status, inspect, dismiss, acknowledge, enable,
  and disable requests against the connection's active session.
- The reusable server client exposes the same advisor request surface, so remote
  clients do not need to construct wire JSON directly.
- Remote TUI clients expose those controls as `/advisor status|inspect|dismiss
  <id>|ack <id>|on|off` and render the server result in chat.
- Unresolved severity-derived blocking notes gate only future risky write, exec,
  publication, scheduling, desktop-control, and MCP tool starts. Read-only tools
  remain available. Acknowledge, dismiss, or session disable releases the gate.
- The check occurs centrally immediately before registry execution, including
  subcalls made through `batch`. It does not cancel or otherwise affect a tool
  that was already running when a note arrived.
- Read-versus-risky classification uses each tool's action and apply fields when
  available. Unknown actions on stateful tools fail closed. DAP evaluation is
  treated as risky because expressions may mutate the debug target.
- A per-session enable overrides the disabled-by-default configuration without
  allocating runtime state while both the configured default and override are
  disabled. Disabling fences active and queued reviews so a late completion
  cannot publish a note or re-establish a blocker.

### Mode contracts

- Interactive mode requests a single concise note only when it materially helps
  the user and avoids unnecessary blocker severity.
- Self-development guardian mode is explicitly tool-less and read-only, and
  checks evaluator integrity, promotion and release claims, scope, safety,
  rollback readiness, and benchmark validity using supplied evidence only.
- Final-review mode requests an independent evidence-referencing verdict against
  the objective and acceptance criteria, and treats missing verification as
  missing rather than inferring success from implementation.

## Validation completed for this slice

Focused tests exercise:

- disabled operation with zero provider calls and no runtime allocation;
- configuration defaults and all mode values;
- exactly-once cursor movement for accepted reviews;
- field, tool, total-input, response, evidence, and private-context bounds;
- recognized-secret redaction on retained input and published notes;
- bounded, redacted tool intent flowing from stored tool calls into advisor
  evidence;
- strict structured-note parsing and graceful malformed-response failure;
- deduplication across consecutive reviewed turns;
- bounded latest-review coalescing while a provider call is active;
- configured review cadence, session-budget exhaustion, and zero-budget
  operation with no provider or runtime cost;
- stale completion fencing across runtime recreation;
- advisor reset on rewind, rewind undo, and compaction application;
- severity-derived urgency and safe-boundary soft-interrupt delivery;
- distinct tool-less contracts for all three configured advisor modes;
- opaque bounded note metadata and blocker release after acknowledge or disable;
- explicit enable override, effective status reporting, and in-flight disable
  publication fencing;
- action-aware risky-tool classification and central enforcement for a mutating
  subcall nested inside `batch`, including proof that the target file is not
  created;
- advisor request/result protocol round trips for every control variant;
- compilation of streaming and non-streaming turn lifecycle integration.

An isolated installed-binary acceptance run also exercised one live server
session through the public socket protocol. It observed one primary provider
turn with the normal tool surface, one tool-less advisor request, retained and
inspected an opaque blocker note, reported effective on/off/on status across
disable and enable, and acknowledged the inspected note successfully.

The focused test and compile commands are recorded in the implementing commit.
This evidence validates the foundation contracts, not the remaining Phase 4
features below.

## Remaining Phase 4 work

### Evidence completeness

- Populate diff summaries through an explicit bounded source.
- Capture new diagnostics and verification results without replaying raw output.
- Include current todo and acceptance-criterion state through stable contracts.

### Lifecycle semantics

- Define and test resume behavior across daemon/process restart.
- Persist handled-note state across daemon/process restart. Current controls and
  note metadata are intentionally live-runtime only.
- Add a handled-note immunity window beyond exact-note deduplication.

### Enforcement and controls

- Refine the conservative risky-tool classification with first-class tool
  capability metadata instead of name-based classification.
- Add non-TUI CLI presentation if a standalone advisor control command is
  needed; the shipped surface is the remote TUI protocol path.
- Persist only the minimum redacted state needed for restart-safe controls.

### Mode-specific behavior

- Complete policy-aware advisor routing across provider permissions and model
  roles.

### Acceptance tests still required

- Resume, rewind, compaction, and reload behavior.
- No concern storms after a note is handled.
- Daemon-restart persistence for inspect, dismiss, acknowledge, and disable.
- Mode-specific real-provider behavior and an evidence-grounded final verdict.
