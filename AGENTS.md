# Repository Guidelines

**Generated:** 2026-09-03T11:37:07Z
**Commit:** f46f9c354
**Branch:** master

## OVERVIEW

jcode is a Rust coding-agent monolith (root `jcode` + 82 `crates/jcode-*`): Ratatui TUI over a long-lived daemon (server/agent/30+ tools) on a shared foundation (auth/providers/session/memory/MCP). Satellites with own toolchains: TS SDK, Cloudflare telemetry worker, iOS app.

## STRUCTURE

```
jcode/
├── src/                  # thin root crate: CLI dispatch + binary entries only
├── crates/
│   ├── jcode-base/       # foundation: auth/provider/config/session/memory/MCP
│   ├── jcode-app-core/   # daemon: server/agent/tool/ambient/overnight
│   ├── jcode-tui/        # Ratatui presentation; re-exports app-core
│   ├── jcode-provider-*/ # pure protocol leaves vs *-runtime live impls
│   └── jcode-*-types/    # stable DTOs only
├── tests/                # root integration + e2e suite
├── scripts/              # build/test gates + budget ratchets (no manifest)
├── sdk/typescript/       # @1jehuang/jcode-sdk (own toolchain)
├── telemetry-worker/     # Cloudflare Worker + D1/R2 (own toolchain)
├── ios/                  # SwiftPM + XcodeGen app (own toolchain)
└── docs/                 # *.md=current; plans/=stale-ok; audits/=snapshots
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| New agent capability | crates/jcode-app-core/src/tool | checklist in crates/jcode-app-core/AGENTS.md |
| Provider add/fix | crates/jcode-base/src/provider, crates/jcode-provider-<name>[-runtime] | pure vs runtime split |
| TUI bug | crates/jcode-tui/src/tui/app | state machine; goldens in app/tests |
| Auth/login | crates/jcode-base/src/auth | test_sandbox; never log secrets |
| Session persistence | crates/jcode-base/src/session | journaling; no-data-loss |
| CLI flags/dispatch | src/cli | args.rs → dispatch.rs → startup.rs |
| E2E/integration | tests/e2e | mock provider; fixtures/openai |
| Quality gates | scripts/check_guardrails.sh | budgets + boundaries + parity |
| TS SDK | sdk/typescript | parity contract with crates/jcode-sdk |
| Telemetry pipeline | telemetry-worker | D1 events + detail tables; R2 transcripts |
| iOS | ios/Sources/JCodeKit | swift test; TestHarness for e2e |
| Current behavior doc | docs/*.md | plans/=forward-looking, may be stale |

## CODE MAP

| Symbol | Type | Location | Role |
|--------|------|----------|------|
| `run` | fn | src/lib.rs:29 | entry: `cli::startup::run` |
| `startup::run` | fn | src/cli/startup.rs | panic hook, logging init, parse, dispatch |
| `pub use jcode_tui::*` | re-export | src/lib.rs:22 | root keeps `crate::<module>` paths |
| `pub use jcode_base::*` | re-export | crates/jcode-app-core/src/lib.rs:24 | app-core keeps base paths |
| workspace | manifest | Cargo.toml:8-93 | `.` + 82 `crates/jcode-*` members |
| default features | manifest | Cargo.toml:214 | `pdf`, `embeddings`, `bedrock` |

## Development Workflow

- **Stay on your own branch** - Do not take, cherry-pick, merge, or copy code from other
  people's or other agents' branches unless the source branch belongs to a repository
  maintainer and the user explicitly asks you to integrate it. Only work from your branch
  and its base (e.g. `main`) otherwise. Never integrate branches owned by non-maintainers
  or other agents yourself; tell the user and let them decide how to proceed.

## Install Notes
- `~/.local/bin/jcode` is the launcher symlink used from `PATH`.
- `~/.jcode/builds/current/jcode` is the active local/source-build channel; self-dev builds and `scripts/install_release.sh` point the launcher here.
- `~/.jcode/builds/stable/jcode` is the stable release channel; `scripts/install.sh` installs this and points the launcher here.
- `~/.jcode/builds/versions/<version>/jcode` stores immutable binaries.
- `~/.jcode/builds/canary/jcode` still exists for canary/testing flows, but it is not the primary self-dev install path.
- On Windows, the equivalents are `%LOCALAPPDATA%\\jcode\\bin\\jcode.exe` for the launcher, `%LOCALAPPDATA%\\jcode\\builds\\stable\\jcode.exe` for stable, and `%LOCALAPPDATA%\\jcode\\builds\\versions\\<version>\\jcode.exe` for immutable installs; `scripts/install.ps1` currently installs the stable channel.
- Ensure `~/.local/bin` is **before** `~/.cargo/bin` in `PATH`.

## Verifying a change at runtime

`cargo build` alone proves nothing about behavior. `jcode run` and interactive
sessions are served by the long-lived daemon at
`~/.jcode/builds/shared-server/jcode`, which is a symlink into
`~/.jcode/builds/versions/<version>/`. Until that symlink is repointed and the
daemon restarted (`jcode self-dev --build`), a freshly built binary is inert and
every runtime check silently measures the old code.

To test a change without disturbing the shared daemon or the caller's session,
run your build against its own socket:

```bash
cargo build --profile selfdev
./target/selfdev/jcode run --no-update --socket /run/user/1000/jcode-mytest.sock '<prompt>'
```

Two things that waste time otherwise:

- `crate::logging::info` writes to a log file, not stderr, so instrumenting a
  code path with it produces no visible output under `--trace`. Use `eprintln!`
  for throwaway diagnostics and delete it before committing.
- Confirm which binary you are actually inspecting. `strings` on
  `builds/shared-server/jcode` reads a 70-byte symlink, not a program; resolve it
  with `readlink -f` first.

## CONVENTIONS

- Build through `scripts/dev_cargo.sh` (memory-sized jobs, host-wide flock gate, fast linker, exports `JCODE_BUILD_GIT_HASH/DATE`). Direct `cargo` falls back to `jobs=4`.
- Trim default stack for probe builds: `JCODE_DEV_FEATURE_PROFILE=minimal|pdf|embeddings|full|default`.
- Quality ratchets must not grow: zero-warning budget, >1200-LOC code/test size budgets, panic budget, swallowed-error budget, wildcard-reexport budget, `check_dependency_boundaries.py`, `cargo machete`, SDK parity test. Aggregate: `scripts/check_guardrails.sh`.
- Keep root re-export compat (`pub use jcode_tui::*` / `pub use jcode_base::*`) when moving modules across the base → app-core → tui spine.
- `*-types` crates carry stable DTOs only (serde + chrono + sibling types; no fs/net/TUI/storage/globals). No new domain DTOs in `jcode-core`.

## ANTI-PATTERNS (THIS PROJECT)

- `--provider claude-subprocess`, Claude CLI shell-out path, `JCODE_USE_CLAUDE_CLI`: deprecated; direct Anthropic API transport is the default.
- Contract crates must not depend on runtime/domain crates; providers must not depend on TUI/server; TUI must not depend on server internals when protocol/client contracts suffice; leaves must not become backdoors into root.
- No mega `jcode-common` crate, no crate-per-directory, no cross-cutting `utils` dumping-ground, no moving UI-adjacent state into core.
- Never `println!` for logging in production code; never-log values (secrets, tokens) must not reach logs/telemetry/traces — enforced by tests.
- `telemetry-worker`: never add columns to `events`; extend detail tables instead.

## COMMANDS

```bash
scripts/check_guardrails.sh                 # full gate: fmt, check, clippy, budgets, machete, parity
scripts/check_guardrails.sh --skip-slow     # skip check/clippy/machete
JCODE_DEV_FEATURE_PROFILE=minimal scripts/test_fast.sh  # lib+bin fast loop
scripts/test_e2e.sh                         # lib suites + --test e2e
```

## NOTES

- No Makefile/Justfile/root package.json; orchestration is `scripts/*.sh` + `.github/workflows/ci.yml`.
- Stock `cargo fmt --check` + `cargo clippy --all-targets --all-features -D warnings` on stable; no rustfmt.toml/clippy.toml/`[lints]`.
- `docs/plans/` is forward-looking and may be stale; `docs/audits/` are point-in-time snapshots; prefer updating an existing doc over adding a near-duplicate.
