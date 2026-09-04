# Phase 3 DAP protocol foundation

The completed foundations cover the DAP protocol, owner-scoped session manager, 30D launch and owned attach, 30E breakpoints and execution control, 30F bounded state inspection, and the 30G opt-in agent-tool, lifecycle, TUI, protocol, and SDK integration. Arbitrary PID attachment remains intentionally unavailable.

## Completion status

**Phase 3 MVP is complete and validated.** There are no remaining Phase 3 implementation blockers.

Finished work:

- 30A through 30C: stable DAP types, framing, protocol classification, asynchronous client behavior, transport shutdown, and owned adapter-process safety.
- 30D: owner-scoped session management, bounded output, explicit `lldb-dap` and native `gdb-dap` launch profiles, and owned spawn-and-attach without caller-supplied PIDs.
- 30E: source breakpoints, thread discovery, execution control, capability checks, revision checks, reconciliation, and lifecycle supervision.
- 30F: bounded stack traces, scopes, variables, evaluation, opaque revision-scoped handles, publication fencing, and admission/termination race closure.
- 30G: opt-in configuration, one server-owned DAP service, the exact 17-action agent tool, owner cleanup and reconnect preservation, bounded `jcode.dap.v1` output, TUI rendering, and Rust and TypeScript SDK propagation.
- Post-MVP: bounded `stepInTargets` discovery and opaque revision-scoped targeted `stepIn` are available through the manager, agent tool, and TUI. Deterministic competitive-eval fixtures cover debugger-led Rust crash localization and targeted step-in repair. Omitted adapter requests select only validated available configured profiles, while explicit unavailable selection fails without fallback.
- Acceptance: focused package, lifecycle, protocol, TUI, SDK, TypeScript, dependency-boundary, binary-build, and isolated runtime-smoke checks pass. The frozen reviewed-v22 Phase 30F gate and two final Phase 30G reviews returned `ADVANCE`.

The remaining items are non-MVP follow-ups listed at the end of this document. The next core OMP roadmap milestone is Phase 4, advisor and independent verification.

## Implemented

- `jcode-dap-types` with stable request, response, event, initialize, capability, and `runInTerminal` wire contracts.
- Extension-tolerant capability decoding that preserves unknown adapter fields.
- `jcode-dap` with bounded `Content-Length` framing and strict DAP message classification.
- An asynchronous client with monotonic request sequences, bounded pending requests, cancellation-safe pending cleanup, out-of-order response correlation, command matching, event delivery, hard end-to-end request deadlines, conditional nonblocking best-effort cancellation, and terminal transport failure propagation.
- Request serialization runs on one bounded blocking lane under the same request deadline. Timeout arithmetic is checked, and unsupported timeout ranges return a structured error instead of panicking.
- Outgoing JSON payloads are bounded to the same 16 MiB protocol limit as incoming frames before a framed message is allocated.
- A bounded dedicated writer actor owns the transport writer. Once a complete encoded frame enters its queue, caller cancellation cannot cancel that frame midway through `write_all` and corrupt subsequent framing.
- Explicit client close and last-client-drop shutdown abort the owned reader and writer tasks, fail pending requests, and release both halves of the transport.
- Event retention is bounded by both 128 events and an 8 MiB serialized-byte ceiling. Events larger than the per-slot 64 KiB ceiling are dropped without terminating the transport, while flooding uses the broadcast channel's deterministic lag signal.
- Reverse adapter requests are observable by callers, then passed to a bounded response actor that preserves outbound sequence order and sends a correctly correlated DAP error response after any active write completes without blocking the reader. Reverse-response queue overflow closes the transport fail-closed. Reverse requests are not executed in this slice.
- A public in-memory fake adapter for protocol and client integration tests.
- An owned adapter-process abstraction with absolute executable and working-directory validation, a controlled allowlisted environment, private process identity, Unix process-group ownership, graceful termination, forced descendant cleanup, bounded stderr retention, and explicit process status.
- Framing compacts consumed bytes once per input batch rather than once per frame, avoiding quadratic behavior for batches containing many small frames.
- Reaped process identities are cleared so later object destruction cannot signal a reused process-group id. Unix termination tolerates the normal already-exited `ESRCH` race and, if group signaling loses a race with natural child exit, waits for and reaps the owned child before forgetting its identity.
- A public `DebugSessionManager` exposes immutable owner-authorized snapshots, lists, bounded output pages, termination, owner cleanup, and shutdown without exposing raw clients, processes, or mutable session entries.
- Opaque process-unique session IDs, canonical workspace keys, one active root per trusted owner, a global active cap, owner reverse indexes, and bounded terminal-history retention are enforced under one short-held registry mutex. Explicit termination retains the owner's active slot until the session publishes `Ended`.
- Session state is validated across reserved, initialization, configuration, running, stopped, terminating, and ended states. Illegal transitions return structured errors, adapter events that race later handshake markers are idempotent, and stale events cannot reactivate terminal sessions.
- Cancellation-safe reservations synchronously release every index and close attached transports when abandoned. A start barrier prevents already-closed attachment from racing supervision before its task handle is stored.
- Per-entry state locks and an async finalization lock keep registry locks out of I/O. Detached owned cleanup tasks make finalization survive caller cancellation, release active ownership, close the client, and use the owned process abstraction for graceful then forced process-group cleanup.
- Supervisors consume output and lifecycle events, observe transport closure and adapter exit, fail closed on receiver lag or non-output source loss, and keep request timeout recoverable.
- Output retention is bounded by event count and UTF-8 bytes, keeps the newest UTF-8-safe tail, advances monotonic cursors through eviction, and reports ring eviction separately from oversized source loss.
- Supervisor tasks hold weak manager references, so dropping the final manager synchronously closes transports and leaves process Drop only as the forced-cleanup backstop.
- Built-in validated `lldb-dap` and native GDB DAP profiles launch only canonical workspace-contained executable files with literal arguments and no shell, environment override, discovery, download, or network behavior. GDB receives only the fixed `--interpreter=dap` adapter argument.
- Adapter omission checks the configured `lldb-dap` profile first and then configured IDs in deterministic order, selecting only a command that resolves to a validated executable. Explicit adapter selection never falls back when unavailable.
- Owned attach spawns and retains the target child internally, authorizes only the owned adapter PID with Linux `PR_SET_PTRACER`, and never accepts a caller-supplied PID.
- Startup uses one checked Tokio deadline across adapter spawn, initialize, launch or attach, initialized, configurationDone, and the start response. Adapter and target ownership enter the cancellation-safe reservation before protocol awaits.
- Finalization asks the live adapter to disconnect within a bound, closes transport, then cleans the owned target and adapter process groups locally. Windows launch and attach fail closed before reservation or spawn.
- A separate non-breaking `DebugOperationConfig` configures bounded operation time, source hashing, breakpoint registries, event reconciliation, thread snapshots, and adapter strings. Exact-30D `DebugSessionManagerConfig` and `StoppedState` remain unchanged.
- Owner-authorized source breakpoints use canonical workspace-contained regular files, exact-byte SHA-256 revisions, full-source `setBreakpoints` replacement, manager-local monotonic IDs, capability gates, bounded public snapshots, compensating clears, and explicit indeterminate synchronization.
- Every breakpoint event is queued in one bounded per-session queue while a transaction is in flight. Response sequence ordering installs adapter IDs before applying higher-sequence ID-only events, while overflow and ambiguous outcomes cannot claim synchronized state.
- Per-entry operation gates serialize breakpoint mutation, ephemeral thread lookup, and execution control without holding synchronous state locks across I/O. Detached operations own the session entry and immutable operation config, never `Arc<ManagerCore>`; terminal closure prevents post-cleanup commits.
- Ephemeral `threads`, `continue`, `pause`, `next`, `stepIn`, and `stepOut` operations enforce owner, state, thread, capability, deadline, and execution-revision checks. Bounded stack traces, scopes, variables, and evaluation use manager-issued revision-scoped handles that expire when execution advances.

## Verified behavior

- Stable DTOs serialize with the required DAP `type`, `request_seq`, camel-case initialize, and `runInTerminal` field shapes.
- Unknown capability fields survive decoding and re-encoding.
- A frame split at every byte boundary decodes correctly.
- Multiple frames, a partial tail, 4,096 frames in one batch, case-insensitive headers, a maximum-sized header with a partial delimiter, malformed headers, duplicate or invalid lengths, and header and payload limits are covered.
- Protocol decoding rejects invalid JSON, unknown message types, non-positive sequences, empty command/event identifiers, and invalid response correlation identifiers.
- Concurrent requests receive out-of-order responses correctly and retain strictly increasing client sequence numbers in actual writer-queue order.
- Events are delivered independently of responses.
- Reverse `runInTerminal` requests are published for observation and receive a fail-closed rejection response with the adapter request sequence and command. A reverse request during outbound backpressure waits outside the reader and receives its correlated rejection after the blocked frame drains. A reverse request followed by EOF terminates the pending request as transport closure rather than hiding EOF behind writer backpressure.
- A request timeout covers bounded serialization, writer-queue admission, writer backpressure, complete frame write and flush, and response waiting under one deadline. A one-millisecond deadline remains prompt while a multi-megabyte request is being serialized, and an unrepresentable timeout returns `InvalidRequestTimeout`. Timeout cleanup removes pending state, and any advertised DAP `cancel` is attempted with `try_send`, so cancellation cannot block or extend the deadline.
- Outgoing messages that exceed the protocol payload limit are rejected as `PayloadTooLarge` before framing.
- Dropping or aborting request futures releases their pending correlation slots, preventing abandoned callers from exhausting the bounded client capacity.
- Aborting a request while its large frame is blocked on a tiny transport still lets the writer actor finish that frame; the following request remains decodable and correlated.
- Oversized events are discarded safely, and a 129-event flood against the 128-event retained channel reports exactly one lagged event without unbounded retention.
- Explicit close fails pending and future requests and closes the fake transport. Dropping non-final clones keeps it open, while dropping the final client closes it automatically.
- Response command mismatches, malformed adapter payloads, EOF, and process exit fail affected pending requests. Reader EOF also interrupts an active blocked write, releases the writer transport, and rejects future requests without waiting for the request deadline.
- The controlled child environment contains only explicitly allowlisted non-secret keys plus the selected `PATH`.
- Non-absolute executable and working-directory inputs are rejected.
- Adapter stderr retains only the configured tail.
- Graceful termination reaps an owned child. Forced cleanup, natural adapter-leader exit, transport-close racing natural adapter exit, and the object-drop backstop remove owned descendant process groups before the group identity is forgotten.
- Owner isolation is verified across list, snapshot, request, output, and terminate. Wrong-owner termination produces no adapter traffic or state change.
- `DapClient`, adapter command/process ownership, and fake transport injection are crate-private; compile-fail doctests prove external callers cannot construct them or send raw `attach` PID JSON.
- Capacity, one-active-per-owner, cancellation release, replacement, terminal pruning, owner cleanup, and shutdown index repair are deterministic.
- Stopped, continued, terminated, exited, stale-event, malformed-event, transport-close, exact broadcast-lag, oversized output, oversized lifecycle event, and already-closed attachment paths are covered.
- Output count, byte, UTF-8 tail, paging, cursor, eviction, and source-loss accounting are covered.
- Recoverable request timeout followed by a successful request, capability-driven cancellation, authorized request round trip, concurrent transport failure plus termination, cancellation of explicit, owner-cleanup, and shutdown callers, termination ownership ordering, attached-reservation cancellation, final-manager-drop closure, and extreme configuration durations are covered.
- Deterministic fake-adapter tests verify both initialized/start-response orders, successful omission of unsupported configurationDone, early-stop preservation, exact disconnect bodies, owner isolation, serialized reservation-drop cleanup, deadline cleanup, scoped Linux ptracer arguments, and owned-target exit during attach. Real framed subprocess tests cover launch, independently self-recorded owned-attach PIDs, fixed native GDB DAP invocation for launch and owned attach, launch and attach rejection with successful retry, target and adapter group cleanup, descendant cleanup after cancellation, disconnect escalation, dead-adapter denial before target spawn, and a reaped target exit that cannot commit a Running session.
- `manager::breakpoints::tests::full_source_set_idempotence_remove_and_exact_revision` proves ordered full-source replacement, idempotence, removal, and exact-byte revision metadata.
- The barrier-driven `manager::breakpoints::tests::reconciliation_contract` tests prove a response can arrive before a higher-sequence ID-only event while the supervisor queues that event before response reconciliation, newer queued events are applied in ascending sequence order, and public queue overflow returns `Indeterminate` without a synchronized claim. `queued_event_at_or_before_response_sequence_is_discarded` covers the inclusive older-event boundary.
- Queue overflow immediately removes synchronized claims from existing sources because a dropped event can no longer be attributed safely.
- `all_unresolved_event_forms_queue_through_supervisor_at_boundary_and_overflow_at_plus_one` drives ID-only, unknown-ID, missing-source, and unknown-source events through the live supervisor transaction path, while `two_source_adapter_id_collision_rejects_and_id_only_events_mutate_neither_source` proves collisions and later ID-only events cannot cross source ownership.
- `manager::breakpoints::tests::ambiguous_timeout_with_unknown_queued_events_is_indeterminate` proves an ambiguous dispatch timeout cannot publish synchronized breakpoint state.
- `manager::breakpoints::tests::source_change_triggers_compensating_empty_clear` proves post-response source mutation causes a compensating empty clear rather than a stale commit.
- `current_operation_source_mutation_between_initial_hash_and_primary_dispatch_emits_no_traffic` proves a current operation revalidates the source before primary dispatch. The paused-Tokio `deadline_contract` matrix uses stage-entry acknowledgements to deterministically exhaust one absolute deadline during primary response wait, bounded worker-thread response validation, post-response source revalidation, compensating clear, and indeterminate reset.
- `manager::breakpoints::tests::wrong_owner_all_breakpoint_apis_emit_zero_traffic` and `manager::control::tests::stale_revision_and_wrong_owner_emit_zero_control_traffic` prove authorization precedes invalid source/thread preprocessing, emits no DAP request, and leaves session state, execution revision, breakpoint registry, and output unchanged.
- `breakpoints::config_tests::validation_boundary_matrix_covers_every_limit_relation_and_timeout` covers every zero operation limit, per-source greater than total, overflowing duration, and the minimal valid configuration. Parser tests cover stopped/continued signed-i32 thread boundaries and legacy missing/null continued bodies.
- Relative breakpoint paths are resolved beneath the canonical workspace root and use canonical paths on the DAP wire. Capability gates reject missing, null, false, and non-boolean values. Unmatched diagnostics saturate at `u64::MAX` without state loss.
- Operational boundary matrices cover exact-limit acceptance and boundary-plus-one rejection for source count, per-source and total breakpoints, all three UTF-8 breakpoint expression fields, canonical source paths, opened-file metadata and observed source bytes, thread count and UTF-8 thread names, queued events, and adapter diagnostic truncation through the public snapshot API. Rejected preflight operations emit zero traffic and preserve prior public state.
- Control race tests prove a stopped event during a fresh pause thread lookup suppresses pause dispatch, malformed successful continue responses conservatively publish Running only when the original revision is current, newer stopped events remain authoritative, and output/breakpoint events do not advance execution revision.
- `manager::breakpoints::tests::exact_30d_public_struct_literals_remain_source_compatible` and `manager::control::tests::public_id_accessors_and_formatting_are_stable` prove exact-30D config/stopped-state construction remains valid and public opaque IDs have stable accessors/formatting.
- `manager::control::tests::threads_are_ephemeral_bounded_and_preserve_order`, `continue_uses_stopped_thread_and_does_not_require_continued_event`, `pause_and_steps_use_exact_commands_and_response_event_order`, and `continue_timeout_is_conservative_and_later_stop_recovers` prove the minimal thread dependency and request/event/revision semantics.
- `manager::lifecycle_tests::manager_drop_closes_transport_with_detached_operation_and_releases_core` proves an aborted public caller leaves a response and breakpoint event queued in detached reconciliation without retaining `ManagerCore`; final manager drop expires the weak core, closes transport, and prevents either queued input from committing state.
- `real_subprocess_full_breakpoint_and_control_contract_repeats_cleanly` runs the framed fake adapter as a real subprocess twice per invocation. It proves Unicode/space source paths, ordered full-set replacement, threads, continue, pause, all three step commands, termination, and process-group cleanup; the focused command was repeated in three independent invocations without a failure.

Focused validation commands:

```text
cargo fmt --manifest-path "$PWD/Cargo.toml" --all -- --check
scripts/dev_cargo.sh test -p jcode-dap-types -p jcode-dap
scripts/dev_cargo.sh clippy -p jcode-dap-types -p jcode-dap --all-targets -- -D warnings
python3 scripts/check_dependency_boundaries.py
cargo tree --manifest-path "$PWD/Cargo.toml" -p jcode-dap-types
cargo tree --manifest-path "$PWD/Cargo.toml" -p jcode-dap
git diff --check
```

All focused DAP checks pass with 423 non-doc tests plus 2 compile-fail doctests across crate-internal, protocol, lifecycle, launch-process, breakpoint/control, state-inspection, and stable-type suites. Low-level client and process tests are crate-internal so those primitives remain unavailable to external callers. The repository-wide code-size and test-size budgets still report unrelated pre-existing drift outside these crates and are not claimed as passing. Every production and test file in `crates/jcode-dap` remains below 1,200 lines, with `manager.rs` at 999 lines.

## Phase 30G agent-tool integration

The operator and agent-facing contract is documented in:

- [`docs/architecture/dap.md`](../../architecture/dap.md)
- [`docs/configuration/dap.md`](../../configuration/dap.md)
- [`docs/tools/dap.md`](../../tools/dap.md)
- [`docs/troubleshooting/dap.md`](../../troubleshooting/dap.md)

The implemented boundary is opt-in and supports configured `kind = "lldb-dap"` and `kind = "gdb-dap"` profiles. Its 18 tool actions cover owned launch and attach, session inspection and cleanup, breakpoints, thread and execution control, bounded stack/step-in-target/scope/variable inspection, and doubly opted-in evaluation. `ToolContext` supplies the trusted owner and canonical workspace. Request JSON cannot override either boundary, and attach never accepts a PID.

Session and inspection identifiers are opaque owner-scoped tokens. Output, protocol messages, request concurrency, retained events, stack frames, scopes, variables, strings, and evaluation results remain bounded. The surface deliberately provides no adapter downloads or discovery, raw DAP request escape hatch, environment override, interactive shell, or cross-process session persistence. A transient transport replacement with a live successor for the same owner preserves the debug session. True owner disconnect, startup timeout or cancellation, adapter failure, explicit owner teardown, server reload, and shutdown converge on owned transport and process-group cleanup.

One server-owned `DapService` supplies the same manager and opaque-token broker to ordinary, recovered, headless, and swarm agent registries. A shared lifecycle gate serializes complete tool actions with owner teardown, shutdown, disconnect cleanup, and resume transitions. Control and breakpoint actions reserve response-token capacity before issuing their DAP mutation. The TUI renders bounded `jcode.dap.v1` results, while the Rust and TypeScript SDKs preserve the existing tool-output string and expose optional bounded presentation metadata rather than adding a policy-bypassing direct DAP API.

Phase 30G focused validation includes:

```text
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh test -p jcode-app-core tool::dap --lib
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh test -p jcode-app-core client_disconnect_cleanup --lib
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh test -p jcode-app-core 'server::client_session::tests::resume_tests' --lib
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh test -p jcode-tui dap --lib
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh check -p jcode-app-core -p jcode-tui
scripts/dev_cargo.sh test -p jcode-protocol
scripts/dev_cargo.sh test -p jcode-harness-api
scripts/dev_cargo.sh test -p jcode-sdk --lib
scripts/dev_cargo.sh test -p jcode-sdk --test client_behavior
cd sdk/typescript && npm run typecheck && npm run build
node --test --experimental-strip-types test/client.test.ts test/protocol.test.ts
python3 scripts/check_dependency_boundaries.py
JCODE_DEV_FEATURE_PROFILE=minimal scripts/dev_cargo.sh build --profile selfdev -p jcode --bin jcode
```

These checks pass, including 6 focused DAP agent-tool tests, the disconnect and resume lifecycle regressions, 82 protocol tests, 16 harness API tests, 12 Rust SDK unit tests, 10 Rust SDK client behavior tests, and 19 focused TypeScript client/protocol tests. The built binary also completed an isolated-socket `jcode run` smoke test. Repository-wide guardrail and strict-clippy results are not claimed because their `fork/master` baseline includes unrelated existing warning, size, panic, swallowed-error, and wildcard-re-export drift; the dependency-boundary check, formatting, focused compilation, and changed-path tests pass.

## Remaining non-MVP work

No item below blocks the completed Phase 3 MVP:

- Adapter discovery beyond the explicit `lldb-dap` and native `gdb-dap` profiles.
- Higher-level debug policy beyond bounded step-in target discovery and the 30F inspection APIs.
- Arbitrary PID attachment.
- Executing reverse `runInTerminal` requests.
- Network, download, or installation behavior.
- Persistent cross-process debug-session recovery.

Before real debugger launch or reverse process requests are enabled on Windows, the runtime needs owned process-tree containment such as a Job Object. The current non-Unix fallback guarantees direct-child cleanup only and does not claim descendant-tree containment.
