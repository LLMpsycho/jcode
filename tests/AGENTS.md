# tests — Root Integration + E2E Suite (Owned by Workspace Cargo)

## OVERVIEW

Workspace-owned integration targets (no own manifest): provider matrices, headed/headless e2e flows, OpenAI fixtures, plus ad-hoc Python checks.

## STRUCTURE

```
tests/
├── e2e/                    # main.rs + session_flow/transport/safety/ambient/burst_spawn/reload_multiclient/binary_integration/provider_behavior/windows_lifecycle
│   ├── mock_provider.rs    # fake provider for hermetic runs
│   └── test_support/       # shared harness helpers
├── fixtures/openai/        # recorded OpenAI payloads
├── auth_login_flow.rs context_window_matrix.rs provider_matrix.rs
└── test_{injection_fix,injection_thorough,selfdev_reload}.py  # ad-hoc pytest checks
```

## CONVENTIONS

- Default runs are hermetic via `mock_provider` + `fixtures/openai`; live-provider coverage is opt-in: `JCODE_REAL_PROVIDER=1` adds `real_provider_smoke.sh`, `JCODE_REAL_AUTH_TEST=1` adds `test_auth_e2e.sh`.
- Put multi-client/reload coverage in `reload_multiclient.rs`, destructive-action coverage in `safety/`; name cases for the invariant (e.g. no-data-loss on crash), not the PR.
- Python files are standalone checks (`python3 -m unittest` / direct run), not part of `cargo test`.

## COMMANDS

```bash
cargo test --test e2e --no-run                 # build only
cargo test --test provider_matrix
cargo test --test e2e
JCODE_REAL_PROVIDER=1 scripts/test_e2e.sh      # + live-provider smoke
```
