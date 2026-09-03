# Phase 3 DAP protocol foundation

This branch implements only the first approved Phase 3 slice from the OMP overtake master plan. It establishes stable Debug Adapter Protocol contracts and a dependency-light runtime foundation. It does not register an agent tool or launch a real debugger session.

## Implemented

- `jcode-dap-types` with stable request, response, event, initialize, capability, and `runInTerminal` wire contracts.
- Extension-tolerant capability decoding that preserves unknown adapter fields.
- `jcode-dap` with bounded `Content-Length` framing and strict DAP message classification.
- An asynchronous client with monotonic request sequences, bounded pending requests, cancellation-safe pending cleanup, out-of-order response correlation, command matching, event delivery, hard end-to-end request deadlines, conditional nonblocking best-effort cancellation, and terminal transport failure propagation.
- A bounded dedicated writer actor owns the transport writer. Once a complete encoded frame enters its queue, caller cancellation cannot cancel that frame midway through `write_all` and corrupt subsequent framing.
- Explicit client close and last-client-drop shutdown abort the owned reader and writer tasks, fail pending requests, and release both halves of the transport.
- Event retention is bounded by both 128 events and an 8 MiB serialized-byte ceiling. Events larger than the per-slot 64 KiB ceiling are dropped without terminating the transport, while flooding uses the broadcast channel's deterministic lag signal.
- Reverse adapter requests are observable by callers, then rejected with a correctly correlated DAP error response. They are not executed in this slice.
- A public in-memory fake adapter for protocol and client integration tests.
- An owned adapter-process abstraction with absolute executable and working-directory validation, a controlled allowlisted environment, private process identity, Unix process-group ownership, graceful termination, forced descendant cleanup, bounded stderr retention, and explicit process status.
- Framing compacts consumed bytes once per input batch rather than once per frame, avoiding quadratic behavior for batches containing many small frames.
- Reaped process identities are cleared so later object destruction cannot signal a reused process-group id. Unix termination tolerates the normal already-exited `ESRCH` race.

## Verified behavior

- Stable DTOs serialize with the required DAP `type`, `request_seq`, camel-case initialize, and `runInTerminal` field shapes.
- Unknown capability fields survive decoding and re-encoding.
- A frame split at every byte boundary decodes correctly.
- Multiple frames, a partial tail, 4,096 frames in one batch, case-insensitive headers, a maximum-sized header with a partial delimiter, malformed headers, duplicate or invalid lengths, and header and payload limits are covered.
- Protocol decoding rejects invalid JSON, unknown message types, non-positive sequences, empty command/event identifiers, and invalid response correlation identifiers.
- Concurrent requests receive out-of-order responses correctly and retain strictly increasing client sequence numbers in actual writer-queue order.
- Events are delivered independently of responses.
- Reverse `runInTerminal` requests are published for observation and receive a fail-closed rejection response with the adapter request sequence and command.
- A request timeout covers writer-queue admission, writer backpressure, complete frame write and flush, and response waiting under one deadline. Timeout cleanup removes pending state, and any advertised DAP `cancel` is attempted with `try_send`, so cancellation cannot block or extend the deadline.
- Dropping or aborting request futures releases their pending correlation slots, preventing abandoned callers from exhausting the bounded client capacity.
- Aborting a request while its large frame is blocked on a tiny transport still lets the writer actor finish that frame; the following request remains decodable and correlated.
- Oversized events are discarded safely, and a 129-event flood against the 128-event retained channel reports exactly one lagged event without unbounded retention.
- Explicit close fails pending and future requests and closes the fake transport. Dropping non-final clones keeps it open, while dropping the final client closes it automatically.
- Response command mismatches, malformed adapter payloads, EOF, and process exit fail affected pending requests. Reader EOF also interrupts an active blocked write, releases the writer transport, and rejects future requests without waiting for the request deadline.
- The controlled child environment contains only explicitly allowlisted non-secret keys plus the selected `PATH`.
- Non-absolute executable and working-directory inputs are rejected.
- Adapter stderr retains only the configured tail.
- Graceful termination reaps an owned child. Forced cleanup, natural adapter-leader exit, and the object-drop backstop remove owned descendant process groups before the group identity is forgotten.

Focused validation commands:

```text
cargo fmt --all -- --check
scripts/dev_cargo.sh test -p jcode-dap-types -p jcode-dap
scripts/dev_cargo.sh clippy -p jcode-dap-types -p jcode-dap --all-targets -- -D warnings
python3 scripts/check_dependency_boundaries.py
cargo tree --manifest-path "$PWD/Cargo.toml" -p jcode-dap-types
cargo tree --manifest-path "$PWD/Cargo.toml" -p jcode-dap
git diff --check
```

All focused DAP checks pass with 35 tests. The repository-wide code-size budget still reports unrelated pre-existing drift outside these crates. The largest DAP production file is 466 lines and the largest DAP test file is 497 lines, so this slice remains below the repository file budgets.

## Deliberately deferred

- Real adapter discovery or debugger launch.
- Agent tool registration and TUI integration.
- Launch, attach, breakpoint, stepping, stack, variable, evaluate, output, and session policy.
- Arbitrary PID attachment.
- Executing reverse `runInTerminal` requests.
- Network, download, or installation behavior.
- Persistent debug-session ownership and authorization checks.
- DAP competitive benchmark tasks.

Before real debugger launch or reverse process requests are enabled on Windows, the runtime needs owned process-tree containment such as a Job Object. The current non-Unix fallback guarantees direct-child cleanup only and does not claim descendant-tree containment.
