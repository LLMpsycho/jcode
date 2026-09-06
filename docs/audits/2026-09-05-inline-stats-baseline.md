# Inline prompt stats: TUI baseline verification

Date: 2026-09-05. Platform: macOS, aarch64. This is a point-in-time audit.

Compared revisions:

- Parent: `0979eaa48` (advisor default change).
- Current: `2d00f5d9a` (move session stats below the prompt).

## Conclusion and limits

All 19 failures in the current full TUI suite also occurred in the parent suite.
No current-only failing test was observed. Both suites still fail; the failures
listed below are unresolved. This is not an all-platform or whole-repository
clean bill of health.

The requested layout outcome was verified through executed buffer tests: context
starts below the input, quotas/cache/model facts appear beneath it, and stats are
not duplicated above the input. The replacement tests exercised eight full-frame
cases and input preservation at 46 widths. The existing 40x8 sticky-prompt preview
test passed on current. An isolated 120x30 live TUI showed input on row 21,
context/quotas on row 22, and model/session details on row 23, with zero reported
layout anomalies.

## Full-suite method and results

Both revisions were tested sequentially in the same disposable detached worktree,
using the same environment and shared Cargo target directory. No checkout or
stash operation was performed in the active working tree.

```sh
COLORTERM=truecolor CARGO_TARGET_DIR="$SHARED_TARGET_DIR"   scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib --   --test-threads=1 --skip test_prompt_entry_shimmer_color_moves_across_positions
```

| Revision | Passed | Failed | Ignored | Filtered | Suite exit |
|---|---:|---:|---:|---:|---:|
| Parent `0979eaa48` | 2246 | 20 | 18 | 1 | 101 |
| Current `2d00f5d9a` | 2242 | 19 | 18 | 1 | 101 |

The comparison process exited 0 because no current-only failures were found,
not because either test suite passed.

## Test-count and skip reconciliation

Exactly 11 tests for the deleted right-side fact-stack renderer were removed,
and six replacement prompt-footer tests were added. All 11 removed tests passed
on parent, and all six replacements ran and passed on current. This accounts
for the net decrease of five executed tests. The same 18 test names were ignored
in both runs, and the same single shimmer test was filtered. No additional test
was silently skipped.

Removed tests:

- `tui::ui::input_ui::tests::right_fact_stack_collision_state_space_is_contiguous_or_hidden`
- `tui::ui::input_ui::tests::right_fact_stack_never_draws_over_the_input_cursor`
- `tui::ui::input_ui::tests::right_fact_stack_never_leaves_an_occupied_row_between_facts`
- `tui::ui::input_ui::tests::right_fact_stack_shifts_up_as_a_unit_when_bottom_row_is_occupied`
- `tui::ui::input_ui::tests::right_fact_stack_treats_styled_blank_cells_as_occupied`
- `tui::ui::tests::swarm_buffer::right_fact_stack_hides_as_a_unit_when_streaming_chrome_cannot_fit_it`
- `tui::ui::tests::swarm_buffer::right_fact_stack_leaves_fully_used_input_rows_untouched_and_moves_up`
- `tui::ui::tests::swarm_buffer::right_fact_stack_shifts_up_when_scheduled_notification_row_is_absent`
- `tui::ui::tests::swarm_buffer::right_fact_stack_survives_narrow_widths_without_overwriting_content`
- `tui::ui::tests::swarm_buffer::right_fact_stack_uses_neutral_gray_except_for_context_usage`
- `tui::ui::tests::swarm_buffer::right_fact_stack_uses_transcript_status_notification_and_input_rows_in_order`

Replacement tests, all passed:

- `tui::ui::tests::swarm_buffer::prompt_stats_are_inline_below_input_without_sidebar_duplicates`
- `tui::ui::tests::swarm_buffer::prompt_stats_do_not_duplicate_during_elastic_or_pinned_overscroll`
- `tui::ui::tests::swarm_buffer::prompt_stats_leave_git_and_compaction_only_sidebars_available`
- `tui::ui::tests::swarm_buffer::prompt_stats_session_facts_use_neutral_gray`
- `tui::ui::tests::swarm_buffer::prompt_stats_survive_narrow_widths_without_overwriting_input`
- `tui::ui::tests::swarm_buffer::prompt_stats_wrap_unicode_and_preserve_usage_preferences`

## Parent-only twentieth failure: intermittent on unchanged parent

`tui::ui::tests::prepared_messages_tests::test_prepare_messages_shows_live_batch_progress_in_chat_history`
exists in both revisions. It was not deleted. It failed in the parent full suite
and passed in the current full suite.

The exact test was then run five times on unchanged parent `0979eaa48`, each in
its own test process with `--exact --test-threads=1`:

| Run | Result | Exit |
|---|---|---:|
| 1 | Failed | 101 |
| 2 | Passed | 0 |
| 3 | Passed | 0 |
| 4 | Passed | 0 |
| 5 | Passed | 0 |

The repeated failure was the same live-batch row-alignment assertion at
`crates/jcode-tui/src/tui/ui_tests/prepare.rs:610`. Its rendered output included
`jcode · perf:reduced` and a collapsed `… 1 completed` row. These observations
establish intermittent behavior on the parent without the stats patch. They do
not establish the root cause, prove timing is responsible, or show that the
patch fixed it. Treat it as an unresolved pre-existing intermittent test.

```sh
COLORTERM=truecolor CARGO_TARGET_DIR="$SHARED_TARGET_DIR"   scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib   tui::ui::tests::prepared_messages_tests::test_prepare_messages_shows_live_batch_progress_in_chat_history   -- --exact --test-threads=1
```

## Independent log validation and retained evidence

A second parser checked every test start/result against the full-suite totals,
compared the exact ignored-name sets, and verified the removed/added function
names against the committed diff. Its first strict pass detected 33 successful
parent test lines containing terminal-title OSC escapes. After stripping OSC
and CSI escapes, every test start had exactly one parsed result and all totals
matched. The failure/test-count conclusions were unchanged.

Local raw logs and machine-readable comparisons are retained in
`~/.jcode/scratch/stats-baseline.UJUXaN/`. This committed audit retains the results,
assertion evidence, test names, reproduction commands and log hashes so the
findings do not depend on scratch retention.

Full-suite log SHA-256:

- `parent.log`: `fb711acdd634c84a82005640b7b835e64c32a04ec15364998c7c53adc92609e3`
- `current.log`: `6fc8661d943288059c15afebee34dad32742ec521d6cdb1bb0e3083ba3818935`

The disposable worktree was removed after verification. No runtime code was
changed by this baseline investigation.

## Nineteen shared unresolved failures

Each entry includes an excerpt from the current full-suite assertion and an
individual reproduction command. Isolated runs may differ from the full suite
because of shared state. These commands have not all been rerun individually.

### tui::app::tests::completed_cycle_rearms_auto_poke_only_when_default_on

```text
thread 'tui::app::tests::completed_cycle_rearms_auto_poke_only_when_default_on' (60264065) panicked at crates/jcode-tui/src/tui/app/tests/remote_events_reload_05.rs:678:9:
assertion failed: !app.schedule_auto_poke_followup_if_needed()
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::completed_cycle_rearms_auto_poke_only_when_default_on -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::recent_project_review_falls_back_cleanly_when_no_repo_is_known

```text
thread 'tui::app::tests::recent_project_review_falls_back_cleanly_when_no_repo_is_known' (60274333) panicked at crates/jcode-tui/src/tui/app/tests/onboarding_flow.rs:1673:5:
assertion failed: app.status_notice.as_ref().is_some_and(|(notice, _)|
        { notice.contains("No recent Git repository found") })
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::recent_project_review_falls_back_cleanly_when_no_repo_is_known -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::test_account_switch_shorthand_switches_openai_account_by_label

```text
thread 'tui::app::tests::test_account_switch_shorthand_switches_openai_account_by_label' (60283319) panicked at crates/jcode-tui/src/tui/app/tests/commands_accounts_02/part_01.rs:547:13:
assertion `left == right` failed
  left: Some("openai-otter")
 right: Some("openai-1")
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::test_account_switch_shorthand_switches_openai_account_by_label -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::test_alignment_status_shows_current_and_saved_defaults

```text
thread 'tui::app::tests::test_alignment_status_shows_current_and_saved_defaults' (60283761) panicked at crates/jcode-tui/src/tui/app/tests/commands_accounts_01/part_02.rs:126:9:
assertion failed: last.content.contains("Alt+C")
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::test_alignment_status_shows_current_and_saved_defaults -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::test_alt_shift_i_toggles_inline_images_and_persists

```text
thread 'tui::app::tests::test_alt_shift_i_toggles_inline_images_and_persists' (60283920) panicked at crates/jcode-tui/src/tui/app/tests/scroll_copy_02/part_02.rs:111:5:
assertion `left == right` failed
  left: Some("Inline images: hidden (⌥+Shift+I to show)")
 right: Some("Inline images: hidden (Alt+Shift+I to show)")
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::test_alt_shift_i_toggles_inline_images_and_persists -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::test_changelog_overlay_mouse_drag_release_copies_text

```text
thread 'tui::app::tests::test_changelog_overlay_mouse_drag_release_copies_text' (60285139) panicked at crates/jcode-tui/src/tui/app/tests/scroll_copy_02/part_01.rs:1479:5:
assertion failed: matches!(app.status_notice().as_deref(), Some("Copied selection") |
    Some("Failed to copy selection") | Some("Selection is empty"))
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::test_changelog_overlay_mouse_drag_release_copies_text -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::test_finish_turn_auto_poke_queues_confidence_summary_when_todos_done

```text
thread 'tui::app::tests::test_finish_turn_auto_poke_queues_confidence_summary_when_todos_done' (60290536) panicked at crates/jcode-tui/src/tui/app/tests/state_model_poke_03.rs:2630:9:
assertion failed: summary.contains("automated follow-up")
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::test_finish_turn_auto_poke_queues_confidence_summary_when_todos_done -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::test_gate_digest_is_delivered_at_turn_end_and_rearms_next_cycle

```text
thread 'tui::app::tests::test_gate_digest_is_delivered_at_turn_end_and_rearms_next_cycle' (60291093) panicked at crates/jcode-tui/src/tui/app/tests/remote_events_reload_05.rs:544:9:
with nothing left outstanding the cycle should finish
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::test_gate_digest_is_delivered_at_turn_end_and_rearms_next_cycle -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::test_improve_mode_persists_in_session_file

```text
thread 'tui::app::tests::test_improve_mode_persists_in_session_file' (60298055) panicked at crates/jcode-tui/src/tui/app/tests/commands_accounts_02/part_02.rs:9:65:
load session: No such file or directory (os error 2)
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::test_improve_mode_persists_in_session_file -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::test_input_composer_drag_selects_and_copies_typed_text

```text
thread 'tui::app::tests::test_input_composer_drag_selects_and_copies_typed_text' (60299195) panicked at crates/jcode-tui/src/tui/app/tests/input_copy_selection.rs:102:5:
assertion `left == right` failed
  left: Some("Copied selection · highlight remains visible")
 right: Some("Copied selection")
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::test_input_composer_drag_selects_and_copies_typed_text -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::test_input_composer_drag_then_release_copies_via_full_mouse_path

```text
thread 'tui::app::tests::test_input_composer_drag_then_release_copies_via_full_mouse_path' (60299247) panicked at crates/jcode-tui/src/tui/app/tests/input_copy_selection.rs:411:5:
drag release over the composer must attempt a copy, got Some("Copied selection · highlight remains visible")
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::test_input_composer_drag_then_release_copies_via_full_mouse_path -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::test_logout_clear_anthropic_accounts_removes_all_accounts_once

```text
thread 'tui::app::tests::test_logout_clear_anthropic_accounts_removes_all_accounts_once' (60300749) panicked at crates/jcode-tui/src/tui/app/tests/state_model_poke_02/part_01.rs:962:61:
called `Result::unwrap()` on an `Err` value: No account with label 'claude-3' found
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::test_logout_clear_anthropic_accounts_removes_all_accounts_once -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::test_reload_preserves_completed_confidence_spike_challenge

```text
thread 'tui::app::tests::test_reload_preserves_completed_confidence_spike_challenge' (60306818) panicked at crates/jcode-tui/src/tui/app/tests/remote_events_reload_05.rs:163:9:
assertion failed: !reloaded_app.schedule_auto_poke_followup_if_needed()
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::test_reload_preserves_completed_confidence_spike_challenge -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::app::tests::unknown_ctrl_chord_sets_hotkey_feedback_with_suggestion

```text
thread 'tui::app::tests::unknown_ctrl_chord_sets_hotkey_feedback_with_suggestion' (60320782) panicked at crates/jcode-tui/src/tui/app/tests/hotkey_feedback_e2e.rs:20:5:
⌨ Ctrl+M isn't bound · nearest: ⌥+M → toggle the side panel
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::app::tests::unknown_ctrl_chord_sets_hotkey_feedback_with_suggestion -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::keybind::tests::new_terminal_alt_enter_binding_parses_and_matches

```text
thread 'tui::keybind::tests::new_terminal_alt_enter_binding_parses_and_matches' (60321173) panicked at crates/jcode-tui/src/tui/keybind.rs:656:9:
assertion `left == right` failed
  left: "⌥+Enter"
 right: "Alt+Enter"
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::keybind::tests::new_terminal_alt_enter_binding_parses_and_matches -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::session_picker::tests::onboarding_banner_renders_prompt_and_both_action_rows

```text
thread 'tui::session_picker::tests::onboarding_banner_renders_prompt_and_both_action_rows' (60321484) panicked at crates/jcode-tui/src/tui/session_picker_tests.rs:1428:5:
blank-session action should stay secondary in the bottom-right: [
    "                                                                                                                        ",
    "                                                                                                                        ",
    "                                                                                                                        ",
    "                                                                                                                        ",
    "                                                                                                                        ",
    "                                                                                                                        ",
    "                                                                                                                        ",
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::session_picker::tests::onboarding_banner_renders_prompt_and_both_action_rows -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::ui::messages::tests::visually_appealing_prompt_batched_retry_renders_complete_todo_card

```text
thread 'tui::ui::messages::tests::visually_appealing_prompt_batched_retry_renders_complete_todo_card' (60322208) panicked at crates/jcode-tui/src/tui/ui_messages/tests.rs:1365:5:
batched todo plan intention was truncated:
  ✓ batch · Inspect starter files and strengthen measurable visual goals · 273 tok
    ✓ todo · Make the visual outcome ob… · 249 tok
      Intent clear: Deliver a single-page vanilla HTML/CSS/JS animation whose pe…
      pelican-bike  ●
        Relevance missing · Coverage missing
        ● Inspect the starter project and determine the page structure · plausib…
    ✓ ls . · 6 tok
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::ui::messages::tests::visually_appealing_prompt_batched_retry_renders_complete_todo_card -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::ui::tests::basic::test_copy_badge_reserves_right_margin_for_info_widgets

```text
thread 'tui::ui::tests::basic::test_copy_badge_reserves_right_margin_for_info_widgets' (60322512) panicked at crates/jcode-tui/src/tui/ui_tests/basic/input_layout.rs:237:5:
assertion `left == right` failed
  left: 18
 right: 16
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::ui::tests::basic::test_copy_badge_reserves_right_margin_for_info_widgets -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.

### tui::ui::tests::basic::test_copy_badge_truncates_full_width_line_before_appending_shortcut

```text
thread 'tui::ui::tests::basic::test_copy_badge_truncates_full_width_line_before_appending_shortcut' (60322513) panicked at crates/jcode-tui/src/tui/ui_tests/basic/input_layout.rs:276:5:
assertion `left == right` failed
  left: 22
 right: 20
```

Reproduce individually: `COLORTERM=truecolor scripts/dev_cargo.sh test --profile selfdev -p jcode-tui --lib tui::ui::tests::basic::test_copy_badge_truncates_full_width_line_before_appending_shortcut -- --exact --test-threads=1`
An individual run may differ from the full suite because of shared state. Preserve the full-suite logs when diagnosing.
