use super::reapply_terminal_modes_to;

#[test]
fn reapply_omits_mouse_sequences_when_capture_is_disabled() {
    let mut output = Vec::new();
    reapply_terminal_modes_to(&mut output, false, true, true).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.starts_with("\x1b[?2004h\x1b[?1004h"));
    assert!(!output.contains("\x1b[?1000h"));
    assert!(output.contains("\x1b[="));
}

#[test]
fn reapply_emits_configured_idempotent_modes_without_keyboard_push() {
    let mut output = Vec::new();
    reapply_terminal_modes_to(&mut output, true, true, true).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("\x1b[?2004h"));
    assert!(output.contains("\x1b[?1004h"));
    assert!(output.contains("\x1b[?1000h"));
    assert!(output.contains("\x1b[="), "must set Kitty keyboard flags");
    assert!(
        !output.contains("\x1b[>"),
        "must not push the Kitty keyboard stack"
    );
}
