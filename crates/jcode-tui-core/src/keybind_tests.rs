use super::*;

/// Decode a kitty CSI-u modifier byte (bitfield + 1) into `KeyModifiers`.
/// This mirrors the sequences we ask Ghostty to forward for Cmd hotkeys, so
/// the test fails if our binding parsing drifts from that wire encoding.
fn kitty_mods(modbyte: u8) -> KeyModifiers {
    let bits = modbyte - 1;
    let mut mods = KeyModifiers::empty();
    if bits & 1 != 0 {
        mods |= KeyModifiers::SHIFT;
    }
    if bits & 2 != 0 {
        mods |= KeyModifiers::ALT;
    }
    if bits & 4 != 0 {
        mods |= KeyModifiers::CONTROL;
    }
    if bits & 8 != 0 {
        mods |= KeyModifiers::SUPER;
    }
    mods
}

#[test]
fn alt_label_uses_option_keycap_on_macos() {
    assert_eq!(alt_label_for_platform(true), MACOS_OPTION_SYMBOL);
    assert_eq!(alt_label_for_platform(false), "Alt");
}

#[test]
fn ghostty_cmd_b_sequence_matches_open_resume_binding() {
    // Ghostty forwards Cmd+B as ESC[98;9u (98='b', super-only).
    let code = KeyCode::Char(char::from_u32(98).unwrap());
    let mods = kitty_mods(9);
    let binding = parse_keybinding("cmd+b").expect("cmd+b parses");
    assert!(
        binding.matches_for_platform(code, mods, true),
        "Cmd+B kitty sequence must trigger the open_resume binding"
    );
}

#[test]
fn ghostty_cmd_shift_semicolon_sequence_matches_new_terminal_binding() {
    // Ghostty forwards Cmd+Shift+; as ESC[59;10u (59=';', shift+super).
    let code = KeyCode::Char(char::from_u32(59).unwrap());
    let mods = kitty_mods(10);
    let binding = parse_keybinding("cmd+shift+;").expect("cmd+shift+; parses");
    assert!(
        binding.matches_for_platform(code, mods, true),
        "Cmd+Shift+; kitty sequence must trigger the new_terminal binding"
    );
}

#[test]
fn legacy_alt_shift_semicolon_sequence_matches_new_terminal_binding() {
    // Legacy terminals may report Alt+Shift+; as the produced ':' character
    // with ALT set but without an explicit SHIFT modifier.
    let binding = parse_keybinding("alt+shift+;").expect("alt+shift+; parses");
    assert!(binding.matches(KeyCode::Char(':'), KeyModifiers::ALT));
    assert!(binding.matches(KeyCode::Char(';'), KeyModifiers::ALT | KeyModifiers::SHIFT));
}

#[test]
fn ctrl_shift_letter_matches_uppercase_and_lowercase_encodings() {
    // Terminals with the Kitty keyboard protocol report Ctrl+Shift+E as
    // either Char('e') or Char('E') with CONTROL|SHIFT. User-configured
    // ctrl+shift+<letter> chords must match both encodings.
    let binding = parse_keybinding("ctrl+shift+e").expect("ctrl+shift+e parses");
    let mods = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    assert!(binding.matches(KeyCode::Char('e'), mods));
    assert!(binding.matches(KeyCode::Char('E'), mods));
    // Plain Ctrl+E (no Shift) must not trigger the shifted binding.
    assert!(!binding.matches(KeyCode::Char('e'), KeyModifiers::CONTROL));
}

fn test_scroll_keys() -> ScrollKeys {
    ScrollKeys {
        up: KeyBinding {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::ALT,
        },
        down: KeyBinding {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::ALT,
        },
        up_fallback: Some(KeyBinding {
            code: KeyCode::Char('K'),
            modifiers: KeyModifiers::SHIFT,
        }),
        down_fallback: Some(KeyBinding {
            code: KeyCode::Char('J'),
            modifiers: KeyModifiers::SHIFT,
        }),
        page_up: KeyBinding {
            code: KeyCode::Char('u'),
            modifiers: KeyModifiers::ALT,
        },
        page_down: KeyBinding {
            code: KeyCode::Char('d'),
            modifiers: KeyModifiers::ALT,
        },
        prompt_up: KeyBinding {
            code: KeyCode::Char('['),
            modifiers: KeyModifiers::ALT,
        },
        prompt_down: KeyBinding {
            code: KeyCode::Char(']'),
            modifiers: KeyModifiers::ALT,
        },
        bookmark: KeyBinding {
            code: KeyCode::Char('g'),
            modifiers: KeyModifiers::CONTROL,
        },
    }
}

#[test]
fn test_scroll_amount_ctrl_fallback() {
    let mut keys = test_scroll_keys();
    keys.up = KeyBinding {
        code: KeyCode::Char('k'),
        modifiers: KeyModifiers::CONTROL,
    };
    keys.down = KeyBinding {
        code: KeyCode::Char('j'),
        modifiers: KeyModifiers::CONTROL,
    };

    assert_eq!(
        keys.scroll_amount(KeyCode::Char('k'), KeyModifiers::CONTROL),
        Some(-3)
    );
    assert_eq!(
        keys.scroll_amount(KeyCode::Char('j'), KeyModifiers::CONTROL),
        Some(3)
    );
}

#[test]
fn test_scroll_amount_ctrl_fallback_disabled_when_rebound() {
    let keys = test_scroll_keys();

    assert_eq!(
        keys.scroll_amount(KeyCode::Char('k'), KeyModifiers::CONTROL),
        None
    );
    assert_eq!(
        keys.scroll_amount(KeyCode::Char('j'), KeyModifiers::CONTROL),
        None
    );
}

#[test]
fn test_scroll_amount_configured_fallback_keys() {
    let keys = test_scroll_keys();

    assert_eq!(
        keys.scroll_amount(KeyCode::Char('K'), KeyModifiers::SHIFT),
        Some(-3)
    );
    assert_eq!(
        keys.scroll_amount(KeyCode::Char('J'), KeyModifiers::SHIFT),
        Some(3)
    );
}

#[test]
fn test_line_scroll_keys_scroll_three_lines() {
    let keys = test_scroll_keys();

    assert_eq!(LINE_SCROLL_AMOUNT, 3);
    assert_eq!(
        keys.scroll_amount(KeyCode::Char('k'), KeyModifiers::ALT),
        Some(-3)
    );
    assert_eq!(
        keys.scroll_amount(KeyCode::Char('j'), KeyModifiers::ALT),
        Some(3)
    );
}

#[test]
fn test_scroll_amount_cmd_jk_not_line_scroll() {
    // Cmd+J / Cmd+K are prompt navigation (see test_prompt_jump_cmd_jk),
    // so they must never be treated as line scrolling on any platform.
    let mut keys = test_scroll_keys();
    keys.up_fallback = None;
    keys.down_fallback = None;

    assert_eq!(
        keys.scroll_amount(KeyCode::Char('k'), KeyModifiers::SUPER),
        None
    );
    assert_eq!(
        keys.scroll_amount(KeyCode::Char('j'), KeyModifiers::SUPER),
        None
    );
}

#[test]
fn test_scroll_amount_cmd_shift_jk_line_scroll() {
    // Cmd+Shift+K / Cmd+Shift+J mirror Ctrl+Shift+K / Ctrl+Shift+J: they
    // line-scroll up / down on macOS regardless of the configured bindings.
    let mut keys = test_scroll_keys();
    keys.up_fallback = None;
    keys.down_fallback = None;

    for code in [KeyCode::Char('k'), KeyCode::Char('K')] {
        assert_eq!(
            keys.scroll_amount(code, KeyModifiers::SUPER | KeyModifiers::SHIFT),
            Some(-LINE_SCROLL_AMOUNT)
        );
    }
    for code in [KeyCode::Char('j'), KeyCode::Char('J')] {
        assert_eq!(
            keys.scroll_amount(code, KeyModifiers::SUPER | KeyModifiers::SHIFT),
            Some(LINE_SCROLL_AMOUNT)
        );
    }
}

#[test]
fn test_scroll_amount_ctrl_shift_jk_line_scroll() {
    // Ctrl+Shift+K / Ctrl+Shift+J line-scroll up / down. This is the shifted
    // counterpart to the un-shifted Ctrl+J/K prompt navigation.
    let mut keys = test_scroll_keys();
    keys.up_fallback = None;
    keys.down_fallback = None;

    for code in [KeyCode::Char('k'), KeyCode::Char('K')] {
        assert_eq!(
            keys.scroll_amount(code, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            Some(-LINE_SCROLL_AMOUNT)
        );
    }
    for code in [KeyCode::Char('j'), KeyCode::Char('J')] {
        assert_eq!(
            keys.scroll_amount(code, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            Some(LINE_SCROLL_AMOUNT)
        );
    }
}

#[test]
fn test_prompt_jump_ctrl_jk() {
    // Ctrl+K / Ctrl+J (un-shifted) move up / down by prompt: the primary
    // default that survives a stock Ghostty + tiling-WM setup.
    let keys = test_scroll_keys();
    assert_eq!(
        keys.prompt_jump(KeyCode::Char('k'), KeyModifiers::CONTROL),
        Some(-1)
    );
    assert_eq!(
        keys.prompt_jump(KeyCode::Char('j'), KeyModifiers::CONTROL),
        Some(1)
    );
}

#[test]
fn test_prompt_jump_shifted_jk_is_not_prompt() {
    // Shifted chords are reserved for incremental scrolling, so they must
    // never be reported as prompt jumps regardless of the modifier family.
    let keys = test_scroll_keys();
    for mods in [
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        KeyModifiers::SUPER | KeyModifiers::SHIFT,
        KeyModifiers::ALT | KeyModifiers::SHIFT,
    ] {
        for code in [
            KeyCode::Char('k'),
            KeyCode::Char('K'),
            KeyCode::Char('j'),
            KeyCode::Char('J'),
        ] {
            assert_eq!(
                keys.prompt_jump(code, mods),
                None,
                "mods={mods:?} code={code:?}"
            );
        }
    }
}

#[test]
fn test_prompt_jump_ctrl_bracket_fallback() {
    let keys = test_scroll_keys();
    assert_eq!(
        keys.prompt_jump(KeyCode::Char('['), KeyModifiers::CONTROL),
        Some(-1)
    );
    assert_eq!(
        keys.prompt_jump(KeyCode::Char(']'), KeyModifiers::CONTROL),
        Some(1)
    );
}

#[test]
fn test_prompt_jump_cmd_bracket_fallback() {
    let keys = test_scroll_keys();
    assert_eq!(
        keys.prompt_jump(KeyCode::Char('['), KeyModifiers::SUPER),
        Some(-1)
    );
    assert_eq!(
        keys.prompt_jump(KeyCode::Char(']'), KeyModifiers::SUPER),
        Some(1)
    );
    assert_eq!(
        keys.prompt_jump(KeyCode::Char('['), KeyModifiers::META),
        Some(-1)
    );
    assert_eq!(
        keys.prompt_jump(KeyCode::Char(']'), KeyModifiers::META),
        Some(1)
    );
}

#[test]
fn test_prompt_jump_cmd_jk() {
    // Cmd+K / Cmd+J move up / down by prompt on macOS (and any terminal that
    // forwards Command as SUPER/META).
    let keys = test_scroll_keys();
    for mods in [KeyModifiers::SUPER, KeyModifiers::META] {
        assert_eq!(keys.prompt_jump(KeyCode::Char('k'), mods), Some(-1));
        assert_eq!(keys.prompt_jump(KeyCode::Char('K'), mods), Some(-1));
        assert_eq!(keys.prompt_jump(KeyCode::Char('j'), mods), Some(1));
        assert_eq!(keys.prompt_jump(KeyCode::Char('J'), mods), Some(1));
    }
}

#[test]
fn test_prompt_jump_option_jk() {
    // Option (Alt) + K / J mirror Cmd+K / Cmd+J for prompt navigation on macOS.
    let keys = test_scroll_keys();
    assert_eq!(
        keys.prompt_jump(KeyCode::Char('k'), KeyModifiers::ALT),
        Some(-1)
    );
    assert_eq!(
        keys.prompt_jump(KeyCode::Char('K'), KeyModifiers::ALT),
        Some(-1)
    );
    assert_eq!(
        keys.prompt_jump(KeyCode::Char('j'), KeyModifiers::ALT),
        Some(1)
    );
    assert_eq!(
        keys.prompt_jump(KeyCode::Char('J'), KeyModifiers::ALT),
        Some(1)
    );
}

#[test]
fn test_prompt_jump_ctrl_digit_reserved_for_rank_jump() {
    let keys = test_scroll_keys();
    assert_eq!(
        keys.prompt_jump(KeyCode::Char('5'), KeyModifiers::CONTROL),
        None
    );
    assert_eq!(
        keys.prompt_jump(KeyCode::Char('4'), KeyModifiers::CONTROL),
        None
    );
}

#[test]
fn test_parse_keybinding_command_and_meta_modifiers() {
    let cmd = parse_keybinding("cmd+j").expect("cmd+j should parse");
    assert_eq!(cmd.code, KeyCode::Char('j'));
    assert!(cmd.modifiers.contains(KeyModifiers::SUPER));

    for raw in ["command+k", "super+k", "win+k", "windows+k"] {
        let binding = parse_keybinding(raw).unwrap_or_else(|| panic!("{raw} should parse"));
        assert_eq!(binding.code, KeyCode::Char('k'));
        assert_eq!(binding.modifiers, KeyModifiers::SUPER);
    }

    let control = parse_keybinding("control+j").expect("control+j should parse");
    assert_eq!(control.code, KeyCode::Char('j'));
    assert_eq!(control.modifiers, KeyModifiers::CONTROL);

    let option_left = parse_keybinding("option+left").expect("option+left should parse");
    assert_eq!(option_left.code, KeyCode::Left);
    assert!(option_left.modifiers.contains(KeyModifiers::ALT));

    let meta = parse_keybinding("meta+k").expect("meta+k should parse");
    assert_eq!(meta.code, KeyCode::Char('k'));
    assert!(meta.modifiers.contains(KeyModifiers::ALT));
}

#[test]
fn key_binding_matches_macos_option_translated_characters() {
    let binding = parse_keybinding("alt+s").expect("alt+s should parse");

    assert!(binding.matches_for_platform(KeyCode::Char('s'), KeyModifiers::ALT, false,));
    assert!(binding.matches_for_platform(KeyCode::Char('ß'), KeyModifiers::empty(), true,));
    assert!(!binding.matches_for_platform(KeyCode::Char('ß'), KeyModifiers::empty(), false,));
}

#[test]
fn macos_option_character_map_covers_default_alt_shortcuts() {
    for (option_char, ascii) in [
        ('å', 'a'),
        ('∫', 'b'),
        ('ç', 'c'),
        ('∂', 'd'),
        ('´', 'e'),
        ('ƒ', 'f'),
        ('©', 'g'),
        ('˙', 'h'),
        ('ˆ', 'i'),
        ('∆', 'j'),
        ('˚', 'k'),
        ('¬', 'l'),
        ('µ', 'm'),
        ('˜', 'n'),
        ('ø', 'o'),
        ('π', 'p'),
        ('œ', 'q'),
        ('®', 'r'),
        ('ß', 's'),
        ('†', 't'),
        ('¨', 'u'),
        ('√', 'v'),
        ('∑', 'w'),
        ('≈', 'x'),
        ('¥', 'y'),
        ('Ω', 'z'),
    ] {
        assert_eq!(
            macos_option_char_to_ascii_key(KeyCode::Char(option_char)),
            Some(ascii)
        );
    }
}

#[test]
fn effort_switch_keys_match_macos_option_arrows_as_alt_arrows() {
    let keys = EffortSwitchKeys {
        increase: parse_keybinding("alt+right").expect("alt+right should parse"),
        decrease: parse_keybinding("alt+left").expect("alt+left should parse"),
    };

    // macOS labels the Alt modifier as Option (⌥). Terminals that forward
    // Option-arrow as an Alt-modified arrow should adjust reasoning effort.
    assert_eq!(
        keys.direction_for(KeyCode::Right, KeyModifiers::ALT),
        Some(1)
    );
    assert_eq!(
        keys.direction_for(KeyCode::Left, KeyModifiers::ALT),
        Some(-1)
    );
    assert_eq!(
        parse_keybinding("option+right")
            .expect("option+right should parse")
            .modifiers,
        KeyModifiers::ALT
    );
}

#[test]
fn effort_switch_keys_match_macos_terminal_option_arrow_escape_encoding() {
    let keys = EffortSwitchKeys {
        increase: parse_keybinding("alt+right").expect("alt+right should parse"),
        decrease: parse_keybinding("alt+left").expect("alt+left should parse"),
    };

    // Terminal.app and many iTerm2 profiles encode Option+Right as ESC+f
    // and Option+Left as ESC+b. Crossterm reports those as Alt+F/B.
    assert_eq!(
        keys.macos_option_arrow_escape_direction_for(KeyCode::Char('f'), KeyModifiers::ALT),
        Some(1)
    );
    assert_eq!(
        keys.macos_option_arrow_escape_direction_for(KeyCode::Char('b'), KeyModifiers::ALT),
        Some(-1)
    );
    assert_eq!(
        keys.macos_option_arrow_escape_direction_for(KeyCode::Char('f'), KeyModifiers::empty()),
        None
    );
}

#[test]
fn effort_switch_keys_do_not_apply_macos_escape_aliases_after_remap() {
    let keys = EffortSwitchKeys {
        increase: parse_keybinding("ctrl+right").expect("ctrl+right should parse"),
        decrease: parse_keybinding("ctrl+left").expect("ctrl+left should parse"),
    };

    assert_eq!(
        keys.macos_option_arrow_escape_direction_for(KeyCode::Char('f'), KeyModifiers::ALT),
        None
    );
    assert_eq!(
        keys.macos_option_arrow_escape_direction_for(KeyCode::Char('b'), KeyModifiers::ALT),
        None
    );
}

#[test]
fn test_parse_function_keybinding_for_copilot_style_keys() {
    let binding = parse_keybinding("ctrl+shift+f23").expect("f23 binding should parse");
    assert_eq!(binding.code, KeyCode::F(23));
    assert!(binding.modifiers.contains(KeyModifiers::CONTROL));
    assert!(binding.modifiers.contains(KeyModifiers::SHIFT));
    assert_eq!(format_binding(&binding), "Ctrl+Shift+F23");
}

#[test]
fn workspace_navigation_keys_match_super_bindings() {
    let keys = WorkspaceNavigationKeys {
        left: vec![KeyBinding {
            code: KeyCode::Char('h'),
            modifiers: KeyModifiers::SUPER,
        }],
        down: vec![KeyBinding {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::SUPER,
        }],
        up: vec![KeyBinding {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::SUPER,
        }],
        right: vec![KeyBinding {
            code: KeyCode::Char('l'),
            modifiers: KeyModifiers::SUPER,
        }],
    };

    assert_eq!(
        keys.direction_for(KeyCode::Char('h'), KeyModifiers::SUPER),
        Some(WorkspaceNavigationDirection::Left)
    );
    assert_eq!(
        keys.direction_for(KeyCode::Char('j'), KeyModifiers::SUPER),
        Some(WorkspaceNavigationDirection::Down)
    );
    assert_eq!(
        keys.direction_for(KeyCode::Char('k'), KeyModifiers::SUPER),
        Some(WorkspaceNavigationDirection::Up)
    );
    assert_eq!(
        keys.direction_for(KeyCode::Char('l'), KeyModifiers::SUPER),
        Some(WorkspaceNavigationDirection::Right)
    );
    assert_eq!(
        keys.direction_for(KeyCode::Char('h'), KeyModifiers::ALT),
        None
    );
}

#[test]
fn workspace_navigation_keys_support_multiple_aliases() {
    let keys = WorkspaceNavigationKeys {
        left: vec![
            KeyBinding {
                code: KeyCode::Char('h'),
                modifiers: KeyModifiers::SUPER,
            },
            KeyBinding {
                code: KeyCode::Left,
                modifiers: KeyModifiers::SUPER,
            },
            KeyBinding {
                code: KeyCode::Left,
                modifiers: KeyModifiers::ALT,
            },
            KeyBinding {
                code: KeyCode::Char('h'),
                modifiers: KeyModifiers::CONTROL,
            },
        ],
        down: vec![
            KeyBinding {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::SUPER,
            },
            KeyBinding {
                code: KeyCode::Down,
                modifiers: KeyModifiers::SUPER,
            },
            KeyBinding {
                code: KeyCode::Down,
                modifiers: KeyModifiers::ALT,
            },
            KeyBinding {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::CONTROL,
            },
        ],
        up: vec![
            KeyBinding {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::SUPER,
            },
            KeyBinding {
                code: KeyCode::Up,
                modifiers: KeyModifiers::SUPER,
            },
            KeyBinding {
                code: KeyCode::Up,
                modifiers: KeyModifiers::ALT,
            },
            KeyBinding {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::CONTROL,
            },
        ],
        right: vec![
            KeyBinding {
                code: KeyCode::Char('l'),
                modifiers: KeyModifiers::SUPER,
            },
            KeyBinding {
                code: KeyCode::Right,
                modifiers: KeyModifiers::SUPER,
            },
            KeyBinding {
                code: KeyCode::Right,
                modifiers: KeyModifiers::ALT,
            },
            KeyBinding {
                code: KeyCode::Char('l'),
                modifiers: KeyModifiers::CONTROL,
            },
        ],
    };

    assert_eq!(
        keys.direction_for(KeyCode::Left, KeyModifiers::SUPER),
        Some(WorkspaceNavigationDirection::Left)
    );
    assert_eq!(
        keys.direction_for(KeyCode::Right, KeyModifiers::ALT),
        Some(WorkspaceNavigationDirection::Right)
    );
    assert_eq!(
        keys.direction_for(KeyCode::Char('j'), KeyModifiers::CONTROL),
        Some(WorkspaceNavigationDirection::Down)
    );
    assert_eq!(
        keys.direction_for(KeyCode::Char('k'), KeyModifiers::CONTROL),
        Some(WorkspaceNavigationDirection::Up)
    );
}
