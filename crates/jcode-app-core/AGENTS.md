# jcode-app-core — Daemon, Agent Loop, Tools

Parent gates (branch, daemon-socket checks, budgets): see root `AGENTS.md`.

## OVERVIEW

Upper application layer: long-lived server, agent turn loop, 30+ tool implementations, ambient/overnight runners.

## STRUCTURE

```
src/
├── server/     # daemon core: lifecycle, client sessions, comm/swarm, reload, debug, sockets
├── tool/       # registry + implementations (bash/edit/batch/browser/memory/MCP/...)
├── agent/      # turn loop: streaming, interrupts, compaction, recovery
├── ambient*    # background agents: manager, scheduler, persistence
├── protocol_tests/  comm_control_tests live beside server/
└── replay.rs   # session replay
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Daemon lifecycle/reload | `src/server/` (104 files: `client_lifecycle.rs`, `swarm.rs`, `reload_*`, `durable_state`) | outage/data-loss risk; check resume + reload matrices |
| New tool | `src/tool/` (80 files) | per-tool module + `*_tests.rs`; trait checklist below |
| macOS computer-use | `src/tool/computer/` (12 files) | moves real cursor; `#[cfg(target_os)]` gated |
| Self-hosting loop | `src/tool/selfdev/` (7 files) | rebuilds jcode itself; never touch shared daemon |
| Turn-loop change | `src/agent/` (15 files) | streaming/interrupt/cancel invariants |
| Background scheduling | `src/ambient*.rs`, `src/ambient/` | idempotency + timing-flake discipline |
| Comm/swarm protocol | `src/server/comm_control_tests/` (21 files) | assign/await/DAG/task-control matrix |

## CONVENTIONS

- New tool: implement `Tool` trait in `src/tool/<name>.rs`, register in registry, add `<name>_tests.rs` beside it; safety-critical tools add a gate module (`bash_destructive_gate`, `discover_secrets` pattern).
- `testdata/` holds webfetch corpora (`ddg_results`, `ddg_anomaly`); update fixtures, never live-fetch in tests.
- Server tests colocate: `client_session_tests/` (clear/reload/resume matrix), `comm_control_tests/` (comm matrix). Name resume cases for the edge, not the fix.
- Ambient scheduler tests must tolerate timing; prefer injected clocks over sleeps.

## ANTI-PATTERNS (THIS CRATE)

- Must not depend on `jcode-tui` or `*-runtime` leaves; server internals stay behind protocol/client contracts.
- No recursive self-dev builds against the shared daemon socket; isolated socket only.
- Deprecated direct-subagent tool path removed — use swarm; do not re-expose it.

## COMMANDS

```bash
cargo test -p jcode-app-core --lib retention_readiness -- --nocapture
cargo test -p jcode-app-core --lib tool::bash::tests::test_stdin_forwarding -- --nocapture
cargo test -p jcode-app-core --lib <module_path>   # targeted; full suite is slow
```
