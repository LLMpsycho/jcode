# Phase 3 DAP protocol foundation

This branch implements only the first approved Phase 3 slice from the OMP overtake master plan. It establishes stable Debug Adapter Protocol contracts and a dependency-light runtime foundation. It does not register an agent tool or launch a real debugger session.

## Implemented

- `jcode-dap-types` with stable request, response, event, initialize, capability, and `runInTerminal` wire contracts.
- Extension-tolerant capability decoding that preserves unknown adapter fields.
- `jcode-dap` with bounded `Content-Length` framing and strict DAP message classification.
- An asynchronous client with monotonic request sequences, bounded pending requests, cancellation-safe pending cleanup, out-of-order response correlation, command matching, event delivery, timeouts, conditional best-effort cancellation, and terminal transport failure propagation.
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
- Concurrent requests receive out-of-order responses correctly and preserve monotonic client sequences.
- Events are delivered independently of responses.
- Reverse `runInTerminal` requests are published for observation and receive a fail-closed rejection response with the adapter request sequence and command.
- Timeouts remove pending state and emit a DAP `cancel` request only after the adapter advertises cancellation support. Cancellation write failure cannot replace the original timeout result.
- Dropping or aborting request futures releases their pending correlation slots, preventing abandoned callers from exhausting the bounded client capacity.
- Response command mismatches, malformed adapter payloads, EOF, and process exit fail affected pending requests. A terminal transport rejects future requests.
- The controlled child environment contains only explicitly allowlisted non-secret keys plus the selected `PATH`.
- Non-absolute executable and working-directory inputs are rejected.
- Adapter stderr retains only the configured tail.
- Graceful termination reaps an owned child. Forced cleanup and the object-drop backstop remove owned descendant process groups.

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

All focused DAP checks pass with 26 tests. The repository-wide code-size budget still reports unrelated pre-existing drift outside these crates. The largest new DAP production file is 325 lines and the largest new DAP test file is 218 lines, so this slice adds no oversized file.

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
