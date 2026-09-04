# DAP tool

The experimental DAP tool gives an agent bounded debugger access to a process launched and owned by the current Jcode session. It is available only when [DAP is enabled](../configuration/dap.md).

## Authority

The tool takes `intent` plus action-specific inputs. It does not take an owner, workspace, raw adapter request, or arbitrary PID. `ToolContext` supplies the owning session and canonical worktree. Every returned session or inspection reference is an opaque token scoped to that owner, session, and stop revision.

## Actions

The tool exposes 18 actions:

| Action | Purpose |
|---|---|
| `launch` | Launch a workspace-contained executable under a configured `lldb-dap` or native `gdb-dap` profile. |
| `attach` | Spawn a target owned by Jcode and attach to that target. No PID is accepted. |
| `sessions` | List the caller's bounded session summaries. |
| `output` | Read a cursor-based page of retained debugger output. |
| `terminate` | Disconnect and clean up one owned session. |
| `set_breakpoint` | Add or return one owner-scoped source breakpoint and its opaque token. |
| `remove_breakpoint` | Remove the breakpoint identified by an opaque breakpoint token. |
| `threads` | Refresh the bounded thread snapshot for a stopped session. |
| `continue` | Continue an authorized stopped thread. |
| `pause` | Pause an authorized running thread. |
| `step_over` | Step over on an authorized stopped thread. |
| `step_in` | Step into on an authorized stopped thread. |
| `step_out` | Step out on an authorized stopped thread. |
| `stack_trace` | Read a bounded page of frames and receive opaque frame tokens. |
| `step_in_targets` | Discover bounded adapter-provided targets for an opaque frame token. |
| `scopes` | Read bounded scopes for an opaque frame token. |
| `variables` | Read a bounded page from an opaque variable token. |
| `evaluate` | Evaluate an expression after global and per-call opt-in. |

Unsupported adapter capabilities produce structured errors rather than raw DAP fallbacks. There is deliberately no `request` action.

When `step_in_targets` returns one or more opaque target tokens, pass one as
`target` to `step_in`. Target tokens are scoped to the owner, debug session,
stack frame, and current execution revision. They expire as soon as execution
advances or debugger state is refreshed at a later stop.

## Launch example

```json
{
  "action": "launch",
  "adapter": "lldb-dap",
  "program": "target/debug/repro",
  "args": ["--case", "invalid-pointer"],
  "cwd": ".",
  "intent": "Reproduce and inspect the invalid pointer"
}
```

Paths are resolved canonically inside the current `ToolContext` workspace. Arguments are literal and never interpreted by a shell.

## Owned attach example

```json
{
  "action": "attach",
  "adapter": "lldb-dap",
  "program": "target/debug/service",
  "args": ["--wait-for-debugger"],
  "cwd": ".",
  "intent": "Inspect startup state in a process owned by this session"
}
```

The request intentionally has no `pid`. Jcode starts the target, retains ownership, and authorizes the adapter to attach only to that child.

## Evaluation example

```json
{
  "action": "evaluate",
  "session_token": "opaque-session-token",
  "frame_token": "opaque-frame-token",
  "expression": "counter",
  "allow_side_effects": true,
  "intent": "Inspect counter with explicit acknowledgement that evaluation may execute code"
}
```

The action is rejected unless `dap.allow_evaluate` is also enabled. Tokens expire when execution resumes, the stop revision changes, the session ends, or ownership changes.

## Output and lifecycle

Use the cursor returned by `output` to request the next page. Retention is bounded, so a page can report evicted events or source loss. Large stack, scope, variable, string, and evaluation responses are rejected or truncated according to their documented bounds rather than returned unbounded.

Sessions are memory-only and server-owned. A transient client transport loss does not dispose the session when a live successor connection resumes the same Jcode owner. A true owner disconnect with no successor, server restart, or reload cleans owned processes and requires a new `launch` or `attach` call.

See [architecture](../architecture/dap.md) and [troubleshooting](../troubleshooting/dap.md).
