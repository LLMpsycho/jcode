# Debug Adapter Protocol architecture

Jcode's Debug Adapter Protocol (DAP) support is an experimental, opt-in debugging subsystem. It keeps protocol and process ownership in `jcode-dap`; the agent-facing tool receives only bounded, owner-authorized values.

## Trust and ownership boundaries

Each tool call derives its authority from `ToolContext` rather than request JSON:

- the owning Jcode session is the DAP owner;
- the canonical current worktree is the workspace boundary;
- callers cannot select another owner or workspace;
- session, frame, scope, and variable references exposed to the model are opaque tokens, not adapter IDs;
- every operation rechecks the owner, session state, stop revision, and token provenance.

A manager permits one active root debug tree per owner and applies a global session cap. An owner cannot inspect, control, read output from, or terminate another owner's session.

## Process boundary

The explicit adapter profiles are `kind = "lldb-dap"` and native `kind = "gdb-dap"`. Jcode starts a configured adapter executable with no shell, and the GDB profile adds only the fixed `--interpreter=dap` argument. Programs, working directories, and breakpoint sources must resolve to canonical regular files inside the current workspace.

`attach` is owned attach, not arbitrary process attachment. Jcode starts the target itself, retains its process identity, and passes only that owned PID to the adapter. The tool schema has no PID input.

Adapter and target processes run in owned process groups with a controlled environment. Jcode does not discover or download adapters, execute raw DAP requests, persist debug sessions, or provide an interactive adapter terminal.

## Protocol and data bounds

DAP traffic uses bounded `Content-Length` framing. Incoming and outgoing JSON payloads are limited to 16 MiB. Pending requests, event queues, reverse-request responses, adapter stderr, retained output, stack frames, scopes, variables, strings, and evaluation results all have explicit limits.

Output is retained as a newest-first-safe UTF-8 tail with monotonic cursors. Pages report both ring eviction and source loss. Oversized output is dropped and counted. Loss of a non-output lifecycle event fails the session closed because state can no longer be trusted.

## Lifecycle

A session moves through reserved, initializing, configuring, running, stopped, terminating, and ended states. Startup uses one deadline across process spawn and the DAP handshake. Operations use bounded deadlines and never hold the registry lock across I/O.

Normal termination requests `disconnect`, closes the transport, then gracefully and finally forcibly cleans the owned target and adapter process groups. Cleanup also runs after timeout, cancellation, transport loss, adapter exit, owner teardown, manager shutdown, or final manager drop.

DAP sessions are in-memory and tied to the live server process. A transient client transport loss does not dispose an owner when a live successor connection resumes the same Jcode session, so the server-owned debug session remains available. A true owner disconnect with no successor, server restart, or reload cleans the owned processes and tokens. Sessions are never reconstructed in another server process.

## Evaluation policy

DAP `evaluate` is potentially mutating. It requires both a global configuration opt-in and an explicit per-call acknowledgement. Read-only inspection remains available without enabling evaluation.

See [configuration](../configuration/dap.md), [tool reference](../tools/dap.md), and [troubleshooting](../troubleshooting/dap.md).
