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
- The initial lifecycle integration populates objective, completed response,
  concise tool summaries, and turn success. The richer diff, diagnostics, todo,
  and acceptance enrichers remain pending.
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
- Notes use the existing soft-interrupt queue. Completed background notes are
  injected before the next user objective reaches the provider, while existing
  mid-turn safe points handle notes that finish during a turn.
- Advisor parsing, routing, request, and stream failures only mark the advisor
  runtime failed. They do not fail or corrupt the primary turn.
- Closing, clearing, or restoring the owner session removes the prior advisor
  runtime. Unique review generations fence late completions from a recreated
  session with the same public session identifier.
- Provider and parser failures are redacted before logging or runtime storage.

## Validation completed for this slice

Focused tests exercise:

- disabled operation with zero provider calls and no runtime allocation;
- configuration defaults and all mode values;
- exactly-once cursor movement for accepted reviews;
- field, tool, total-input, response, evidence, and private-context bounds;
- recognized-secret redaction on retained input and published notes;
- strict structured-note parsing and graceful malformed-response failure;
- deduplication across consecutive reviewed turns;
- bounded latest-review coalescing while a provider call is active;
- stale completion fencing across runtime recreation;
- severity-derived urgency and safe-boundary soft-interrupt delivery;
- compilation of streaming and non-streaming turn lifecycle integration.

The focused test and compile commands are recorded in the implementing commit.
This evidence validates the foundation contracts, not the remaining Phase 4
features below.

## Remaining Phase 4 work

### Evidence completeness

- Populate diff summaries through an explicit bounded source.
- Capture new diagnostics and verification results without replaying raw output.
- Include current todo and acceptance-criterion state through stable contracts.
- Preserve tool intent as well as tool name and concise result.

### Lifecycle semantics

- Define and test resume behavior across daemon/process restart.
- Reset or re-prime the cursor and private context on rewind and compaction.
- Add a handled-note acknowledgement and immunity window, rather than relying
  only on exact-note deduplication.
- Add explicit call-rate and session-budget policy beyond one in-flight review
  and the per-turn publication limit.

### Enforcement and controls

- Gate only future destructive, write, and exec tools for unresolved blocker
  notes. Never interrupt an atomic publication already in progress.
- Add inspect, dismiss, acknowledge, and disable controls in the CLI/TUI and
  protocol surfaces.
- Persist only the minimum state needed for those controls and never persist
  unredacted advisor evidence.

### Mode-specific behavior

- Make interactive notes visible without unnecessary interruption.
- Enforce the read-only self-development guardian contract and evaluator,
  promotion, scope, safety, rollback, and benchmark-integrity checks.
- Implement final-review mode with an evidence-referencing independent verdict.
- Complete policy-aware advisor routing across provider permissions and model
  roles.

### Acceptance tests still required

- Resume, rewind, compaction, and reload behavior.
- Blocker gating limited to future risky tools and release after acknowledgement.
- No concern storms after a note is handled.
- User inspect, dismiss, acknowledge, and disable flows.
- Mode-specific real-provider behavior and an evidence-grounded final verdict.
