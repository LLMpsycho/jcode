#[test]
fn handterm_native_latex_cell_symbol_moves_invokes_and_restores_cursor() {
    assert_eq!(
        super::handterm_native_latex_cell_symbol(r"\frac{a}{b}", 3, 2).as_deref(),
        Some("\x1b[s\x1b[2A\x1b[1D\x1b_L;\\frac{a}{b}\x1b\\\x1b[u\x1b[1C")
    );
    assert_eq!(
        super::handterm_native_latex_cell_symbol("x", 1, 1).as_deref(),
        Some("\x1b[s\x1b_L;x\x1b\\\x1b[u\x1b[1C")
    );
    assert!(super::handterm_native_latex_cell_symbol("x\x1by", 1, 1).is_none());
}

#[test]
fn tail_follow_small_appends_snap_to_bottom() {
    // Streaming-sized appends (<= min jump) snap directly; no animation.
    crate::tui::ui::set_last_resolved_chat_scroll(100);
    let scroll = super::resolve_tail_follow_scroll(103, 30);
    assert_eq!(scroll, 103);
    assert!(!crate::tui::ui::tail_catchup_active());
}

#[test]
fn tail_follow_large_append_slides_in_bounded_steps() {
    // A 12-row jump advances by at most TAIL_CATCHUP_MAX_STEP per frame
    // and reports an active catch-up until it reaches the bottom.
    crate::tui::ui::set_last_resolved_chat_scroll(100);
    let first = super::resolve_tail_follow_scroll(112, 30);
    assert!(first < 112, "must not snap: {first}");
    assert!(
        first - 100 <= super::TAIL_CATCHUP_MAX_STEP,
        "step bounded: {first}"
    );
    assert!(crate::tui::ui::tail_catchup_active());

    // Subsequent frames converge to the bottom and clear the flag.
    let mut scroll = first;
    let mut guard = 0;
    while scroll < 112 {
        crate::tui::ui::set_last_resolved_chat_scroll(scroll);
        scroll = super::resolve_tail_follow_scroll(112, 30);
        guard += 1;
        assert!(guard < 50, "catch-up must converge");
    }
    assert_eq!(scroll, 112);
    assert!(!crate::tui::ui::tail_catchup_active());
}

#[test]
fn explicit_tail_follow_request_skips_content_catch_up_animation() {
    crate::tui::ui::set_last_resolved_chat_scroll(40);
    crate::tui::ui::request_tail_follow_snap();

    let scroll = super::resolve_tail_follow_scroll(80, 30);

    assert_eq!(scroll, 80);
    assert!(!crate::tui::ui::tail_catchup_active());
}

#[test]
fn tail_follow_caps_lag_to_one_viewport() {
    // A huge append (way beyond a screen) starts at most one viewport
    // behind the bottom so the catch-up never replays pages of content.
    crate::tui::ui::set_last_resolved_chat_scroll(100);
    let scroll = super::resolve_tail_follow_scroll(400, 30);
    assert!(scroll >= 400 - 30, "lag capped to viewport: {scroll}");
    assert!(crate::tui::ui::tail_catchup_active());
}

#[test]
fn tail_follow_backward_motion_snaps() {
    // Content shrank (commit collapsed reasoning): snap, don't animate.
    crate::tui::ui::set_last_resolved_chat_scroll(100);
    let scroll = super::resolve_tail_follow_scroll(80, 30);
    assert_eq!(scroll, 80);
    assert!(!crate::tui::ui::tail_catchup_active());
}

#[test]
fn default_copy_badge_alt_label_matches_platform() {
    #[cfg(target_os = "macos")]
    assert_eq!(super::copy_badge_alt_label_from_config(""), "⌥");

    #[cfg(not(target_os = "macos"))]
    assert_eq!(super::copy_badge_alt_label_from_config(""), "Alt");
}

#[test]
fn copy_badge_alt_label_uses_trimmed_config_override() {
    assert_eq!(
        super::copy_badge_alt_label_from_config(" Option "),
        "Option"
    );
    assert_eq!(super::copy_badge_alt_label_from_config("⌥"), "⌥");
}
