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
- Reaped process identities are cleared so later object destruction cannot signal a reused process-group id. Unix termination tolerates the normal already-exited `ESRCH` race.
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
- Graceful termination reaps an owned child. Forced cleanup, natural adapter-leader exit, and the object-drop backstop remove owned descendant process groups before the group identity is forgotten.
- Owner isolation is verified across list, snapshot, request, output, and terminate. Wrong-owner termination produces no adapter traffic or state change.
- `DapClient`, adapter command/process ownership, and fake transport injection are crate-private; compile-fail doctests prove external callers cannot construct them or send raw `attach` PID JSON.
- Capacity, one-active-per-owner, cancellation release, replacement, terminal pruning, owner cleanup, and shutdown index repair are deterministic.
- Stopped, continued, terminated, exited, stale-event, malformed-event, transport-close, exact broadcast-lag, oversized output, oversized lifecycle event, and already-closed attachment paths are covered.
- Output count, byte, UTF-8 tail, paging, cursor, eviction, and source-loss accounting are covered.
- Recoverable request timeout followed by a successful request, capability-driven cancellation, authorized request round trip, concurrent transport failure plus termination, cancellation of explicit, owner-cleanup, and shutdown callers, termination ownership ordering, attached-reservation cancellation, final-manager-drop closure, and extreme configuration durations are covered.
- Deterministic fake-adapter tests verify both initialized/start-response orders, successful omission of unsupported configurationDone, early-stop preservation, exact disconnect bodies, owner isolation, serialized reservation-drop cleanup, deadline cleanup, scoped Linux ptracer arguments, and owned-target exit during attach. Real framed subprocess tests cover launch, independently self-recorded owned-attach PIDs, launch and attach rejection with successful retry, target and adapter group cleanup, descendant cleanup after cancellation, disconnect escalation, dead-adapter denial before target spawn, and a reaped target exit that cannot commit a Running session.
- `manager::breakpoints::tests::full_source_set_idempotence_remove_and_exact_revision` proves ordered full-source replacement, idempotence, removal, and exact-byte revision metadata.
- `manager::breakpoints::tests::id_only_higher_sequence_event_is_applied_after_response` proves the corrected response-delivery race: a higher-sequence ID-only event is queued and applied after adapter-ID installation.
- `manager::breakpoints::tests::queue_all_events_is_bounded_and_overflow_is_recorded` proves unknown-source and ID-only events share the bounded in-flight queue and overflow is explicit.
- `manager::breakpoints::tests::ambiguous_timeout_with_unknown_queued_events_is_indeterminate` proves an ambiguous dispatch timeout cannot publish synchronized breakpoint state.
- `manager::breakpoints::tests::source_change_triggers_compensating_empty_clear` proves post-response source mutation causes a compensating empty clear rather than a stale commit.
- `manager::breakpoints::tests::wrong_owner_all_breakpoint_apis_emit_zero_traffic` and `manager::control::tests::stale_revision_and_wrong_owner_emit_zero_control_traffic` prove universal owner denial with no DAP request.
- `manager::breakpoints::tests::exact_30d_public_struct_literals_remain_source_compatible` and `manager::control::tests::public_id_accessors_and_formatting_are_stable` prove exact-30D config/stopped-state construction remains valid and public opaque IDs have stable accessors/formatting.
- `manager::control::tests::threads_are_ephemeral_bounded_and_preserve_order`, `continue_uses_stopped_thread_and_does_not_require_continued_event`, `pause_and_steps_use_exact_commands_and_response_event_order`, and `continue_timeout_is_conservative_and_later_stop_recovers` prove the minimal thread dependency and request/event/revision semantics.
- `manager::control::tests::final_manager_drop_closes_transport_despite_detached_operation` proves an aborted public caller leaves reconciliation detached without retaining `ManagerCore`, so final manager drop still closes transport promptly.
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

All focused DAP checks pass with 127 tests plus 2 compile-fail doctests: 102 crate-internal library/client/process tests, 1 repeated breakpoint/control subprocess test, 10 framing/protocol integration tests, 11 launch-process tests, and 3 DAP type tests. Low-level client and process tests are crate-internal so those primitives remain unavailable to external callers. The repository-wide code-size budget still reports unrelated pre-existing drift outside these crates. Every production and test file in `crates/jcode-dap` remains below 1,200 lines.

## Deliberately deferred

- Adapter discovery and debugger profiles beyond configured `lldb-dap`.
- Agent tool registration and TUI integration.
- Stack trace, frame selection, scopes, variables, evaluate, step-in targets, and higher-level debug policy.
- Arbitrary PID attachment.
- Executing reverse `runInTerminal` requests.
- Network, download, or installation behavior.
- App-core ownership wiring, agent-facing operations, and persistent cross-process debug-session recovery.
- DAP competitive benchmark tasks.

Before real debugger launch or reverse process requests are enabled on Windows, the runtime needs owned process-tree containment such as a Job Object. The current non-Unix fallback guarantees direct-child cleanup only and does not claim descendant-tree containment.
