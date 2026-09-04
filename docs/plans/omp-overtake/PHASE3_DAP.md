# Phase 3 DAP protocol foundation

This branch implements the DAP protocol foundation, owner-scoped session manager, the Phase 3 30D launch and owned-attach slice, and the 30E breakpoint/thread/execution-control slice. It does not register an agent tool or expose arbitrary PID attachment.

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
- A built-in validated `lldb-dap` profile launches only canonical workspace-contained executable files with literal arguments and no shell, environment override, discovery, download, or network behavior.
- Owned attach spawns and retains the target child internally, authorizes only the owned adapter PID with Linux `PR_SET_PTRACER`, and never accepts a caller-supplied PID.
- Startup uses one checked Tokio deadline across adapter spawn, initialize, launch or attach, initialized, configurationDone, and the start response. Adapter and target ownership enter the cancellation-safe reservation before protocol awaits.
- Finalization asks the live adapter to disconnect within a bound, closes transport, then cleans the owned target and adapter process groups locally. Windows launch and attach fail closed before reservation or spawn.
- A separate non-breaking `DebugOperationConfig` configures bounded operation time, source hashing, breakpoint registries, event reconciliation, thread snapshots, and adapter strings. Exact-30D `DebugSessionManagerConfig` and `StoppedState` remain unchanged.
- Owner-authorized source breakpoints use canonical workspace-contained regular files, exact-byte SHA-256 revisions, full-source `setBreakpoints` replacement, manager-local monotonic IDs, capability gates, bounded public snapshots, compensating clears, and explicit indeterminate synchronization.
- Every breakpoint event is queued in one bounded per-session queue while a transaction is in flight. Response sequence ordering installs adapter IDs before applying higher-sequence ID-only events, while overflow and ambiguous outcomes cannot claim synchronized state.
- Per-entry operation gates serialize breakpoint mutation, ephemeral thread lookup, and execution control without holding synchronous state locks across I/O. Detached operations own the session entry and immutable operation config, never `Arc<ManagerCore>`; terminal closure prevents post-cleanup commits.
- Ephemeral `threads`, `continue`, `pause`, `next`, `stepIn`, and `stepOut` operations enforce owner, state, thread, capability, deadline, and execution-revision checks. No stack, frame, scope, variable, or evaluation cache exists.

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
- Deterministic fake-adapter tests verify both initialized/start-response orders, successful omission of unsupported configurationDone, early-stop preservation, exact disconnect bodies, owner isolation, serialized reservation-drop cleanup, deadline cleanup, scoped Linux ptracer arguments, and owned-target exit during attach. Real framed subprocess tests cover launch, independently self-recorded owned-attach PIDs, launch and attach rejection with successful retry, target and adapter group cleanup, descendant cleanup after cancellation, disconnect escalation, dead-adapter denial before target spawn, and a reaped target exit that cannot commit a Running session.
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

All focused DAP checks pass with 211 non-doc tests plus 2 compile-fail doctests: 174 crate-internal library/client/process tests, 12 Phase 30E contract/lifecycle subprocess tests, 1 repeated breakpoint/control subprocess test, 10 framing/protocol integration tests, 11 launch-process tests, and 3 DAP type tests. Low-level client and process tests are crate-internal so those primitives remain unavailable to external callers. The repository-wide code-size and test-size budgets still report unrelated pre-existing drift outside these crates and are not claimed as passing. Every production and test file in `crates/jcode-dap` remains below 1,200 lines, with `manager.rs` at 999 lines.

## Current status and unfinished work

Phase 30E is accepted at commit `9acbd29db55955121564c352c8aa7b228172fb68` with tree `c440734898c86ab60b3e6f7548d9b17add722da2`. Phase 30F state inspection has not entered the repository yet. The reviewed-v18 contract, checklist, executable acceptance gate, manifest, and deterministic bundle are sealed in scratch, but implementation remains blocked until two fresh independent reviews both return `ADVANCE`.

The following work is not finished:

- Obtain two independent `ADVANCE` verdicts for the sealed reviewed-v18 Phase 30F authority. Any blocking finding requires a newly named immutable revision and another full review.
- Create the Phase 30F implementation worktree and branch from the exact accepted Phase 30E commit without changing that accepted base.
- Make the first commit a behavior-preserving manager-construction and session-entry initialization extraction. This creates line-count headroom while keeping existing public APIs unchanged.
- Add the exact additive inspection configuration, opaque handles, request/result DTOs, error and evaluate-outcome types, manager constructor, and four owner-scoped operations for stack traces, scopes, variables, and evaluate.
- Correct the existing initialize wire keys to `clientID` and `adapterID`, advertise client variable paging and variable-type support, and preserve the accepted public initialize type shape.
- Refactor the DAP client around one positive-`int32` outbound allocator and a tracked request path that distinguishes queue admission from physical write, preserves ordinary request compatibility, drains late responses safely, and isolates replacement client instances.
- Add reusable publication and lifecycle fencing for revision changes, target or adapter exit, transport loss, owner shutdown, client replacement, caller drop, deadline settlement, and response publication.
- Implement bounded stack, scope, variable, and evaluate response parsing with one operation deadline, retained blocking permits, strict DAP numeric domains, aggregate text limits, frame and variable provenance, cross-thread frame-ID uniqueness, paging rules, and no raw adapter identifiers in public results.
- Add every canonical deterministic race, boundary, lifecycle, confidentiality, and compatibility test. The current binding inventory requires exactly 177 unit/integration tests and 21 real-subprocess cases, with each subprocess case repeated three times by the final gate.
- Run formatting, focused all-target tests and doctests, strict Clippy, dependency-boundary and dependency-tree checks, diff and scope checks, public-surface checks, local line-count gates, exact inventory mapping, subprocess cleanup evidence, and honest BASE-versus-HEAD global-budget qualification.
- Commit each dependency-ordered slice and run the sealed final acceptance gate from a clean implementation worktree. Phase 30F is complete only when the full gate passes and independent review accepts the exact implementation commit.

## Remaining implementation order

1. Accept the reviewed-v18 binding authority through two independent reviews.
2. Extract manager construction and session-entry initialization without behavior changes.
3. Add the public inspection API, configuration, initialize-wire compatibility, and constructor plumbing.
4. Add shared outbound sequence allocation, tracked admission/correlation, bounded decode, and deterministic client test barriers.
5. Add manager inspection preflight, thread acquisition, publication fencing, lifecycle settlement, and exact client-instance checks.
6. Implement stack trace, scopes, variables, and evaluate in that order.
7. Complete deterministic race and boundary coverage, then the real-subprocess acceptance matrix.
8. Run the complete reviewed acceptance gate, resolve every failure, and bind the accepted Phase 30F commit and tree in this plan.

## Deferred after Phase 30F

- Adapter discovery and debugger profiles beyond configured `lldb-dap`.
- Agent tool registration and TUI integration.
- Step-in targets and higher-level debug policy beyond the bounded Phase 30F inspection surface.
- Arbitrary PID attachment.
- Executing reverse `runInTerminal` requests.
- Network, download, or installation behavior.
- App-core ownership wiring, agent-facing operations, and persistent cross-process debug-session recovery.
- DAP competitive benchmark tasks.

Before real debugger launch or reverse process requests are enabled on Windows, the runtime needs owned process-tree containment such as a Job Object. The current non-Unix fallback guarantees direct-child cleanup only and does not claim descendant-tree containment.
