# jcode-base — Foundation: Auth, Providers, Session, Config

Parent gates (branch, daemon-socket checks, budgets): see root `AGENTS.md`.

## OVERVIEW

Bottom layer everything builds on: multi-provider auth + routing, session persistence, config, memory, MCP, usage accounting.

## STRUCTURE

```
src/
├── auth/       # OAuth/login/refresh coordinator/doctor/stores (37 files)
├── provider/   # routing, failover, catalog, pricing, selection, startup (40 files)
├── session/    # persistence, journal, crash, maintenance, paths (9 files)
├── config/     # layered file/defaults/env-overrides + change reports (6 files)
├── mcp/        # client/manager/pool/protocol/schema-cache (9 files)
├── usage/      # accounting, display, provider fetch, API keys (8 files)
└── memory.rs session.rs gateway.rs ...  # single-file subsystems + *_tests.rs
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Login/OAuth flow | `src/auth/` | credential sources + `test_sandbox`; `oauth_tests/` helpers |
| Add/route provider | `src/provider/` | catalog + route builders; `tests/` covers failover/resolution |
| Session storage | `src/session/` + `src/session_tests/` | journaling; env-guarded cases |
| Config precedence | `src/config/` | file < defaults < env; change-report on edits |
| MCP compat | `src/mcp/` | pool + schema-cache rules |
| Spend/billing display | `src/usage/` | mock provider fetch in tests |

## CONVENTIONS

- Secrets: never log tokens/credentials (never-log list enforced by tests); auth tests run inside `test_sandbox`.
- Provider work: pure formatting/parsing lives in `crates/jcode-provider-<name>`; live transport in `crates/jcode-provider-<name>-runtime`; only routing/selection/catalog glue lives here.
- Session changes must preserve the journal + crash-recovery path; run `session_tests` env-guarded cases.
- Config edits ship with precedence (file/defaults/env) + display-summary + change-report updates.

## ANTI-PATTERNS (THIS CRATE)

- Must not depend on any `*-runtime` crate (dev-only `openrouter-runtime` routing-test exception) or on `jcode-tui*`.
- `jcode-provider-env` must not depend on auth — register via `register_api_key_fallback_resolver`.
- Claude CLI subprocess transport (`claude.rs` shim, `JCODE_USE_CLAUDE_CLI`) is deprecated; do not extend.
- Auth-lifecycle parity with `jcode-provider-doctor` drivers is sync-tested; do not import doctor drivers here.

## COMMANDS

```bash
cargo test -p jcode-base --lib secret_input -- --nocapture
cargo test -p jcode-base --lib <module_path>   # targeted (auth/provider/session/config)
```
