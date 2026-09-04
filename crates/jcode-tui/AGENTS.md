# jcode-tui — Ratatui Presentation Layer

Parent gates (branch, daemon-socket checks, budgets): see root `AGENTS.md`.

## OVERVIEW

Terminal UI only: event loop + state machine in `src/tui/app`, render primitives in `src/tui/ui*`; re-exports app-core so `crate::<module>` paths keep working.

## STRUCTURE

```
src/
├── lib.rs            # re-exports jcode_app_core; owns tui + video_export
└── tui/
    ├── app/          # state machine: commands, auth, input, remote, lifecycle (156 files)
    │   ├── remote/   # remote-daemon client: events, reconnect, queue recovery (10 files)
    │   └── tests/    # issue regressions + golden fixtures (58 files, 13 *_01/ dirs)
    ├── session_picker/  # filter/navigation/render/loading split (6 files)
    └── ui_tests/     # visual harness: rendering, palette, diagrams, swarm buffer
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| State/event bug | `src/tui/app/` (`tui_state`, `state_ui_*`, `commands_*`) | biggest files: `inline_interactive.rs`, `input.rs`, `commands.rs`, `auth.rs` |
| Remote/reconnect bug | `src/tui/app/remote/` | ordering + idempotency + queue-recovery invariants |
| Add regression test | `src/tui/app/tests/issue_*` | name for the issue; new `*_01/` fixture dir only for a new scenario class |
| Visual regression | `src/tui/ui_tests/` (`basic/`, `diagrams/`) | golden-update policy below |
| Session picker | `src/tui/session_picker/` | self-contained filter→navigation→render flow |

## CONVENTIONS

- Keep `state_ui_*` (render) separate from `commands_*` (mutations) and lifecycle; raw `crossterm::Event` stays at the boundary, never in core state.
- Golden fixtures: reuse an existing `*_01/` scenario dir when possible; adding one needs a genuinely new event sequence.
- UI tests run single-threaded; CI skips two known-flaky shimmer/prompt cases — mirror those skips locally before blaming your change.

## ANTI-PATTERNS (THIS CRATE)

- Must not depend on server internals when protocol/client contracts suffice.
- Do not proliferate golden fixture dirs per fix; extend the matching `issue_*` test.
- `#[deprecated]` shims in `info_widget.rs` point at `calculate_placements` / `render_all` — use the replacements.

## COMMANDS

```bash
cargo test -p jcode-tui --lib onboarding_graph::
COLORTERM=truecolor cargo test -p jcode-tui --lib -- --test-threads=1 --skip test_prompt_entry_shimmer_color_moves_across_positions --skip right_fact_stack_uses_neutral_gray_except_for_context_usage
```
