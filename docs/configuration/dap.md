# DAP configuration

DAP is experimental and disabled by default. Enabling it is an explicit local configuration decision.

## Example

```toml
[dap]
enabled = true
allow_evaluate = false

[dap.adapters.lldb-dap]
kind = "lldb-dap"
command = "/absolute/path/to/lldb-dap"

[dap.adapters.gdb]
kind = "gdb-dap"
command = "/absolute/path/to/gdb"
```

The adapter command must already exist and be executable. Relative commands are resolved against the server's `PATH`; use an absolute command path when deterministic resolution is required. Jcode does not auto-discover candidates, install, update, or download debug adapters.

The supported explicit adapter kinds are `lldb-dap` and GDB's native `gdb-dap` mode. Jcode starts GDB with the fixed `--interpreter=dap` argument. Program and working-directory values are resolved under the `ToolContext` workspace, and user-provided launch arguments are passed literally without a shell. Adapter environment overrides and adapter command arguments are not exposed.

When a DAP tool request omits `adapter`, Jcode selects only from configured adapters whose command currently resolves to a validated executable. It preserves the historical `lldb-dap` preference, then checks the remaining configured IDs in deterministic order. An explicitly requested unavailable adapter never falls back to another profile and returns configuration guidance instead.

## Evaluation requires two opt-ins

Setting `allow_evaluate = true` only makes the action eligible. Each `evaluate` call must also include its explicit execution acknowledgement. This double opt-in exists because an expression can call functions or otherwise mutate the debuggee.

Leave `allow_evaluate = false` when only stack, scope, and variable inspection is needed.

## Limits and timeouts

The subsystem applies bounded defaults for:

- active sessions and one active root per owner;
- startup and individual operation deadlines;
- DAP payloads, pending requests, and event queues;
- retained output event count, retained UTF-8 bytes, and output page size;
- adapter stderr retention;
- breakpoint sources and breakpoints per source;
- thread snapshots, stack frames, step-in targets, scopes, and variables per response;
- string fields, evaluate expressions, and evaluate results.

Invalid or excessive limits fail configuration rather than silently disabling bounds. Output pages may be shorter than requested and include loss metadata when older data was evicted or oversized source events were discarded.

## Disable and rollback

Set `dap.enabled = false` or remove the `[dap]` section, then restart or reload Jcode. Active sessions are not persisted. Shutdown attempts a bounded DAP disconnect and locally cleans all owned adapter and target process groups.

See [architecture](../architecture/dap.md), [tool reference](../tools/dap.md), and [troubleshooting](../troubleshooting/dap.md).
