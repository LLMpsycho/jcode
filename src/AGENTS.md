# src — Root Crate: CLI Dispatch + Binary Entries

Parent gates (branch, daemon-socket checks, budgets): see root `AGENTS.md`.

## OVERVIEW

Thin entry layer only: `jcode` binary parses CLI and dispatches into `jcode-tui` / `jcode-app-core` via re-exported paths.

## STRUCTURE

```
src/
├── main.rs   # jcode binary: allocator tuning → cli::startup::run
├── lib.rs    # pub use jcode_tui::* shim + pub mod cli + run()
├── cli/      # args.rs → dispatch.rs → startup.rs; login/, provider_init.rs
└── bin/      # harness, test_api + dev-bins benches/probes (feature-gated)
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| New flag/subcommand | `cli/args.rs` then `cli/dispatch.rs` | Clap derive; keep dispatch arms thin |
| Startup/login flow | `cli/startup.rs`, `cli/login.rs`, `cli/provider_init.rs` | runtime provider registration lives here |
| Headless harness | `bin/harness.rs` | `#[tokio::main]` entry |
| Dev benches/probes | `bin/*_bench.rs`, `bin/mermaid_side_panel_probe.rs` | `dev-bins` feature only |

## CONVENTIONS

- `crate::<module>` paths resolve through the `pub use jcode_tui::*` shim — do not add local modules shadowing re-exported names.
- CLI tests colocate: `cli/login/tests.rs`, `cli/dispatch_tests.rs`; run with the `test-support` feature unification the workspace already wires.
- Keep this crate thin: presentation logic → `jcode-tui`, orchestration → `jcode-app-core`, foundation → `jcode-base`.

## COMMANDS

```bash
cargo test --lib --bin jcode -- cli::   # targeted CLI tests
cargo test --bin jcode --no-run   # build test target only
```
