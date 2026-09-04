# Troubleshooting DAP

## The DAP tool is unavailable

DAP is opt-in. Confirm that `[dap] enabled = true` is present and restart or reload Jcode. If configuration was changed while a session was active, start a new session because debug sessions are not persisted or reconnected.

## The configured adapter cannot be started

Configure `kind = "lldb-dap"` for LLDB or `kind = "gdb-dap"` for a GDB build with native DAP support, and set `command` to an existing executable. Relative commands use the server's `PATH`; use an absolute command path for deterministic resolution. Jcode does not auto-discover, install, or download adapters.

## A program, CWD, or source path is rejected

All such paths must canonicalize inside the current `ToolContext` worktree. Symlink escapes, missing files, non-regular program/source files, and paths in a different checkout are rejected. Run or rebuild the target in the current workspace.

## Attach asks for a PID

It should not. DAP supports only owned attach: provide the target program and literal arguments, and Jcode spawns the child before attaching. Arbitrary PID attachment and cross-user process access are intentionally unavailable.

## A session or inspection token is invalid

Tokens are opaque and owner-scoped. Frame, scope, and variable tokens are also tied to a particular stop revision. They become stale after continue, stepping, another stop, termination, owner cleanup, or server reload. A transient transport reconnect to the same live owner does not by itself invalidate them. Refresh `sessions` or `threads`, then request a new stack trace and descend through fresh tokens when lifecycle state changed.

## Evaluate is denied

Evaluation has two gates. Enable `dap.allow_evaluate`, restart or reload Jcode, and provide the action's explicit side-effect acknowledgement. This restriction applies even to expressions that appear read-only because adapters may execute code while evaluating them.

## Output is missing or a page reports loss

Output retention is bounded by event count and UTF-8 bytes. Older entries may be evicted, and oversized output events are dropped and counted. Continue from the returned monotonic cursor. Lost non-output lifecycle events terminate the session because Jcode cannot safely infer debugger state.

## A request times out

Startup has one deadline across adapter spawn, initialize, launch or attach, configuration, and the start response. Individual operations also have deadlines. Inspect the retained adapter stderr and session end reason, then verify that the adapter and debuggee are responsive. A timed-out request does not grant permission to issue a raw DAP request.

## The session ended after disconnect

A true owner disconnect with no live successor triggers bounded disconnect plus local owned-process cleanup. A transient client transport loss does not dispose the server-owned session when a successor connection resumes the same Jcode owner. Server reload, adapter exit, transport failure, timeout, owner cleanup, and shutdown also end the session. Sessions and opaque tokens are never persisted across server processes, so start a new session after those terminal events.

## A child process remains

Wait briefly for graceful cleanup, then check the session end reason and adapter stderr. Jcode escalates from DAP `disconnect` to local process-group termination. Report a reproducible survivor as a bug, including the platform, adapter version, action, and sanitized end reason. Do not include secrets, arbitrary process data, or raw debugger memory.

See [configuration](../configuration/dap.md), [tool reference](../tools/dap.md), and [architecture](../architecture/dap.md).
