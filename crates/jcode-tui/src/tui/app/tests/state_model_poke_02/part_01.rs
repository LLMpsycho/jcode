#[test]
fn test_side_diagram_uses_left_splitter_instead_of_rounded_box() {
    let _lock = scroll_render_test_lock();
    let mut app = create_test_app();
    app.diagram_mode = crate::config::DiagramDisplayMode::Pinned;
    app.diagram_pane_enabled = true;
    app.diagram_pane_position = crate::config::DiagramPanePosition::Side;

    crate::tui::mermaid::clear_active_diagrams();
    crate::tui::mermaid::register_active_diagram(0x444, 900, 450, Some("side".to_string()));

    let backend = ratatui::backend::TestBackend::new(120, 40);
    let mut terminal = ratatui::Terminal::new(backend).expect("failed to create terminal");
    let text = render_and_snap(&app, &mut terminal);

    let diagram_area = crate::tui::ui::last_layout_snapshot()
        .and_then(|layout| layout.diagram_area)
        .expect("expected side diagram area after render");
    let buf = terminal.backend().buffer();

    assert_eq!(buf[(diagram_area.x, diagram_area.y)].symbol(), "│");
    assert_eq!(buf[(diagram_area.x, diagram_area.y + 1)].symbol(), "│");
    assert!(text.contains("pinned 1/1"), "rendered text: {text}");

    crate::tui::mermaid::clear_active_diagrams();
}

#[test]
fn test_tool_side_panel_focus_supports_horizontal_pan_keys() {
    let mut app = create_test_app();
    app.diff_mode = crate::config::DiffDisplayMode::Inline;
    app.side_panel = crate::side_panel::SidePanelSnapshot {
        focused_page_id: Some("plan".to_string()),
        pages: vec![crate::side_panel::SidePanelPage {
            id: "plan".to_string(),
            title: "Plan".to_string(),
            file_path: "".to_string(),
            format: crate::side_panel::SidePanelPageFormat::Markdown,
            source: crate::side_panel::SidePanelPageSource::Managed,
            content: "hello".to_string(),
            updated_at_ms: 1,
        }],
    };

    assert!(app.handle_diagram_ctrl_key(KeyCode::Char('l'), false));
    assert!(app.diff_pane_focus);

    app.handle_key(KeyCode::Right, KeyModifiers::empty())
        .unwrap();
    assert_eq!(app.diff_pane_scroll_x, 4);
    assert!(app.input.is_empty());

    app.handle_key(KeyCode::Left, KeyModifiers::empty())
        .unwrap();
    assert_eq!(app.diff_pane_scroll_x, 0);
}

#[test]
fn test_tool_side_panel_focus_supports_image_zoom_keys() {
    let mut app = create_test_app();
    app.diff_mode = crate::config::DiffDisplayMode::Inline;
    app.side_panel = crate::side_panel::SidePanelSnapshot {
        focused_page_id: Some("plan".to_string()),
        pages: vec![crate::side_panel::SidePanelPage {
            id: "plan".to_string(),
            title: "Plan".to_string(),
            file_path: "".to_string(),
            format: crate::side_panel::SidePanelPageFormat::Markdown,
            source: crate::side_panel::SidePanelPageSource::Managed,
            content: "hello".to_string(),
            updated_at_ms: 1,
        }],
    };

    assert!(app.handle_diagram_ctrl_key(KeyCode::Char('l'), false));
    assert!(app.diff_pane_focus);

    app.handle_key(KeyCode::Char('+'), KeyModifiers::empty())
        .unwrap();
    assert_eq!(app.side_panel_image_zoom_percent, 110);

    app.handle_key(KeyCode::Char('-'), KeyModifiers::empty())
        .unwrap();
    assert_eq!(app.side_panel_image_zoom_percent, 100);

    app.handle_key(KeyCode::Char('+'), KeyModifiers::empty())
        .unwrap();
    app.handle_key(KeyCode::Char('0'), KeyModifiers::empty())
        .unwrap();
    assert_eq!(app.side_panel_image_zoom_percent, 100);
}

#[test]
fn test_mouse_horizontal_scroll_over_tool_side_panel_pans_without_focus_change() {
    let mut app = create_test_app();
    app.diff_mode = crate::config::DiffDisplayMode::Inline;
    app.diff_pane_scroll_x = 0;
    app.diff_pane_focus = false;
    app.side_panel = crate::side_panel::SidePanelSnapshot {
        focused_page_id: Some("plan".to_string()),
        pages: vec![crate::side_panel::SidePanelPage {
            id: "plan".to_string(),
            title: "Plan".to_string(),
            file_path: "".to_string(),
            format: crate::side_panel::SidePanelPageFormat::Markdown,
            source: crate::side_panel::SidePanelPageSource::Managed,
            content: "hello".to_string(),
            updated_at_ms: 1,
        }],
    };

    crate::tui::ui::record_layout_snapshot(
        Rect::new(0, 0, 40, 20),
        None,
        Some(Rect::new(40, 0, 20, 20)),
        None,
    );

    let scroll_only = app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::ScrollRight,
        column: 45,
        row: 5,
        modifiers: KeyModifiers::empty(),
    });

    assert!(
        !scroll_only,
        "side-panel horizontal pan should request an immediate redraw"
    );
    assert_eq!(app.diff_pane_scroll_x, 1);
    assert!(!app.diff_pane_focus);
}

#[test]
fn test_ctrl_mouse_scroll_over_tool_side_panel_zooms_images() {
    let mut app = create_test_app();
    app.diff_mode = crate::config::DiffDisplayMode::Inline;
    app.side_panel_image_zoom_percent = 100;
    app.diff_pane_focus = false;
    app.side_panel = crate::side_panel::SidePanelSnapshot {
        focused_page_id: Some("plan".to_string()),
        pages: vec![crate::side_panel::SidePanelPage {
            id: "plan".to_string(),
            title: "Plan".to_string(),
            file_path: "".to_string(),
            format: crate::side_panel::SidePanelPageFormat::Markdown,
            source: crate::side_panel::SidePanelPageSource::Managed,
            content: "hello".to_string(),
            updated_at_ms: 1,
        }],
    };

    crate::tui::ui::record_layout_snapshot(
        Rect::new(0, 0, 40, 20),
        None,
        Some(Rect::new(40, 0, 20, 20)),
        None,
    );

    let scroll_only = app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 45,
        row: 5,
        modifiers: KeyModifiers::CONTROL,
    });

    assert!(
        !scroll_only,
        "side-panel image zoom should request an immediate redraw"
    );
    assert_eq!(app.side_panel_image_zoom_percent, 110);
    assert!(!app.diff_pane_focus);
}

#[test]
fn test_mouse_scroll_events_are_classified_as_scroll_only() {
    let mut app = create_test_app();
    app.diff_mode = crate::config::DiffDisplayMode::File;

    crate::tui::ui::record_layout_snapshot(
        Rect::new(0, 0, 40, 20),
        None,
        Some(Rect::new(40, 0, 20, 20)),
        None,
    );

    let scroll_only = app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 45,
        row: 5,
        modifiers: KeyModifiers::empty(),
    });

    assert!(
        scroll_only,
        "scroll wheel events should be deferrable during streaming"
    );

    let non_scroll = app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: 5,
        modifiers: KeyModifiers::empty(),
    });

    assert!(!non_scroll, "clicks should still redraw immediately");
}

#[test]
fn test_handterm_native_scroll_command_updates_chat_offset() {
    // Use an app with real scrollable content and draw it, so the renderer
    // records a non-zero max scroll. Since the phantom-offset fix,
    // scroll_down treats a rendered max of 0 (e.g. an undrawn or empty
    // transcript) as "already at the bottom" and snaps back to follow mode.
    let (mut app, mut terminal) = create_scroll_test_app(50, 12, 0, 24);
    app.auto_scroll_paused = true;
    app.scroll_offset = 6;
    terminal
        .draw(|f| crate::tui::ui::draw(f, &app))
        .expect("draw failed");
    crate::tui::ui::record_layout_snapshot(Rect::new(0, 0, 50, 12), None, None, None);
    assert!(
        crate::tui::ui::last_max_scroll() > 7,
        "scroll test content should exceed the viewport"
    );

    app.apply_handterm_native_scroll(super::handterm_native_scroll::HostToApp::Scroll {
        pane: super::handterm_native_scroll::PaneKind::Chat,
        delta: -2,
    });
    assert_eq!(
        app.scroll_offset, 5,
        "the first row should render immediately"
    );
    assert_eq!(
        app.mouse_scroll_queue, -1,
        "the second row should remain queued"
    );
    app.progress_mouse_scroll_animation();
    assert_eq!(app.scroll_offset, 4);

    app.apply_handterm_native_scroll(super::handterm_native_scroll::HostToApp::Scroll {
        pane: super::handterm_native_scroll::PaneKind::Chat,
        delta: 3,
    });
    assert_eq!(
        app.scroll_offset, 5,
        "the first row should render immediately"
    );
    assert_eq!(
        app.mouse_scroll_queue, 2,
        "later rows should animate on ticks"
    );
    app.progress_mouse_scroll_animation();
    assert_eq!(
        app.scroll_offset, 6,
        "the queued rows should be revealed separately"
    );
    assert_eq!(app.mouse_scroll_queue, 1);
    app.progress_mouse_scroll_animation();
    assert_eq!(app.scroll_offset, 7);
}

#[cfg(unix)]
#[test]
fn test_handterm_native_scroll_client_roundtrips_over_socket() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;

    let _lock = crate::storage::lock_test_env();
    let _render_lock = scroll_render_test_lock();
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("handterm-scroll.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind unix listener");
    unsafe {
        std::env::set_var("HANDTERM_NATIVE_SCROLL_SOCKET", &socket_path);
    }

    let mut client = super::handterm_native_scroll::HandtermNativeScrollClient::connect_from_env()
        .expect("native scroll client should connect from env");
    let (mut server, _) = listener.accept().expect("accept client");
    server
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("set read timeout");

    let (mut app, mut terminal) = create_scroll_test_app(50, 12, 0, 24);
    app.auto_scroll_paused = true;
    app.scroll_offset = 6;
    let _ = render_and_snap(&app, &mut terminal);

    client.sync_from_app(&app);

    let mut buf = [0u8; 4096];
    let n = server.read(&mut buf).expect("read pane snapshot");
    let line = std::str::from_utf8(&buf[..n]).expect("utf8 snapshot");
    assert!(line.contains("pane_snapshot"));
    assert!(line.contains("chat"));
    assert!(line.contains("\"position\":6"));

    server
        .write_all(b"{\"type\":\"scroll\",\"pane\":\"chat\",\"delta\":-2}\n")
        .expect("write host scroll command");

    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let command = runtime
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(1), client.recv())
                .await
                .expect("timeout waiting for scroll command")
        })
        .expect("scroll command should arrive");

    app.apply_handterm_native_scroll(command);
    assert_eq!(app.scroll_offset, 5);
    assert_eq!(app.mouse_scroll_queue, -1);
    app.progress_mouse_scroll_animation();
    assert_eq!(app.scroll_offset, 4);

    unsafe {
        std::env::remove_var("HANDTERM_NATIVE_SCROLL_SOCKET");
    }
}

#[test]
fn test_mouse_scroll_help_overlay_updates_help_scroll() {
    let mut app = create_test_app();
    app.help_scroll = Some(5);

    let scroll_only = app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 10,
        row: 5,
        modifiers: KeyModifiers::empty(),
    });

    assert!(
        scroll_only,
        "help overlay mouse wheel should be scroll-only"
    );
    assert_eq!(app.help_scroll, Some(8));

    let scroll_only = app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 10,
        row: 5,
        modifiers: KeyModifiers::empty(),
    });

    assert!(scroll_only);
    assert_eq!(app.help_scroll, Some(5));
}

#[test]
fn test_mouse_scroll_changelog_overlay_updates_changelog_scroll() {
    let mut app = create_test_app();
    app.changelog_scroll = Some(2);

    let scroll_only = app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 10,
        row: 5,
        modifiers: KeyModifiers::empty(),
    });

    assert!(
        scroll_only,
        "changelog overlay mouse wheel should be scroll-only"
    );
    assert_eq!(app.changelog_scroll, Some(0));

    let scroll_only = app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 10,
        row: 5,
        modifiers: KeyModifiers::empty(),
    });

    assert!(scroll_only);
    assert_eq!(app.changelog_scroll, Some(3));
}

#[test]
fn test_mouse_scroll_over_diagram_pans_hovered_pane_without_changing_focus() {
    let _render_lock = scroll_render_test_lock();
    let (mut app, mut terminal) = create_scroll_test_app(120, 30, 0, 80);
    app.diagram_mode = crate::config::DiagramDisplayMode::Pinned;
    app.diagram_pane_enabled = true;
    app.diagram_pane_position = crate::config::DiagramPanePosition::Side;
    app.diagram_pane_ratio = 40;
    app.diagram_pane_ratio_from = 40;
    app.diagram_pane_ratio_target = 40;
    app.diagram_pane_anim_start = None;
    app.diagram_focus = false;

    crate::tui::mermaid::clear_active_diagrams();
    crate::tui::mermaid::register_active_diagram(0x444, 900, 450, None);
    let _ = render_and_snap(&app, &mut terminal);
    let max_scroll = crate::tui::ui::last_max_scroll();
    assert!(max_scroll > 2, "expected scrollable chat content");
    for (position, messages_area, diagram_area) in [
        (
            crate::config::DiagramPanePosition::Side,
            Rect::new(0, 0, 80, 30),
            Rect::new(80, 0, 40, 30),
        ),
        (
            crate::config::DiagramPanePosition::Top,
            Rect::new(0, 12, 120, 18),
            Rect::new(0, 0, 120, 12),
        ),
    ] {
        app.diagram_pane_position = position;
        crate::tui::ui::record_layout_snapshot(messages_area, Some(diagram_area), None, None);
        for focused in [false, true] {
            for (column, row) in [
                (diagram_area.x, diagram_area.y),
                (diagram_area.x + 10, diagram_area.y + 5),
                (diagram_area.right() - 1, diagram_area.bottom() - 1),
            ] {
                for (kind, expected) in [
                    (MouseEventKind::ScrollUp, (5, 4)),
                    (MouseEventKind::ScrollDown, (5, 6)),
                    (MouseEventKind::ScrollLeft, (4, 5)),
                    (MouseEventKind::ScrollRight, (6, 5)),
                ] {
                    app.diagram_focus = focused;
                    app.diagram_scroll_x = 5;
                    app.diagram_scroll_y = 5;

                    let scroll_only = app.handle_mouse_event(MouseEvent {
                        kind,
                        column,
                        row,
                        modifiers: KeyModifiers::empty(),
                    });

                    assert!(scroll_only, "diagram wheel should be scroll-only");
                    assert_eq!((app.diagram_scroll_x, app.diagram_scroll_y), expected);
                    assert_eq!(app.diagram_focus, focused, "hover must not steal focus");
                    assert!(
                        !app.auto_scroll_paused,
                        "hover must not pause chat auto-scroll"
                    );
                    assert_eq!(app.scroll_offset, 0, "hover must not scroll chat");
                    assert_eq!(app.mouse_scroll_queue, 0);
                    assert_eq!(app.mouse_scroll_target, None);
                    assert_eq!(app.diagram_pane_ratio, 40);
                    assert_eq!(app.diagram_pane_ratio_from, 40);
                    assert_eq!(app.diagram_pane_ratio_target, 40);
                    assert!(app.diagram_pane_anim_start.is_none());
                }
            }
        }
    }

    crate::tui::mermaid::clear_active_diagrams();
}

#[test]
fn test_mouse_scroll_over_chat_ignores_diagram_keyboard_focus() {
    let _render_lock = scroll_render_test_lock();
    let (mut app, mut terminal) = create_scroll_test_app(120, 30, 0, 80);
    app.diagram_mode = crate::config::DiagramDisplayMode::Pinned;
    app.diagram_pane_enabled = true;
    app.diagram_pane_position = crate::config::DiagramPanePosition::Side;
    app.diagram_focus = true;

    crate::tui::mermaid::register_active_diagram(0x447, 900, 450, None);
    let _ = render_and_snap(&app, &mut terminal);
    assert!(crate::tui::ui::last_max_scroll() > 2);
    crate::tui::ui::record_layout_snapshot(
        Rect::new(0, 0, 80, 30),
        Some(Rect::new(80, 0, 40, 30)),
        None,
        None,
    );
    app.diagram_scroll_x = 5;
    app.diagram_scroll_y = 5;

    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 79,
        row: 10,
        modifiers: KeyModifiers::empty(),
    });

    assert!(app.auto_scroll_paused);
    assert_ne!(app.scroll_offset, 0);
    assert_eq!((app.diagram_scroll_x, app.diagram_scroll_y), (5, 5));
    assert!(app.diagram_focus, "wheel must not change keyboard focus");

    crate::tui::mermaid::clear_active_diagrams();
}

#[test]
fn test_mouse_scroll_over_diagram_at_top_does_not_scroll_chat() {
    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    app.diagram_mode = crate::config::DiagramDisplayMode::Pinned;
    app.diagram_pane_enabled = true;
    app.diagram_pane_position = crate::config::DiagramPanePosition::Side;
    app.diagram_focus = true;
    app.diagram_scroll_y = 0;

    crate::tui::mermaid::clear_active_diagrams();
    crate::tui::mermaid::register_active_diagram(0x446, 900, 450, None);
    crate::tui::ui::record_layout_snapshot(
        Rect::new(0, 0, 80, 30),
        Some(Rect::new(80, 0, 40, 30)),
        None,
        None,
    );

    for focused in [false, true] {
        app.diagram_focus = focused;
        let scroll_only = app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 90,
            row: 10,
            modifiers: KeyModifiers::empty(),
        });

        assert!(
            scroll_only,
            "hovered diagram owns wheel events even at its limit"
        );
        assert_eq!(app.diagram_scroll_y, 0);
        assert_eq!(app.diagram_focus, focused);
        assert_eq!(
            app.mouse_scroll_target, None,
            "must not enqueue chat scrolling"
        );
    }

    crate::tui::mermaid::clear_active_diagrams();
}

#[test]
fn test_dragging_diagram_border_resizes_immediately_without_animation() {
    let mut app = create_test_app();
    app.diagram_mode = crate::config::DiagramDisplayMode::Pinned;
    app.diagram_pane_enabled = true;
    app.diagram_pane_position = crate::config::DiagramPanePosition::Side;
    app.diagram_pane_ratio = 40;
    app.diagram_pane_ratio_from = 40;
    app.diagram_pane_ratio_target = 40;
    app.diagram_pane_anim_start = Some(Instant::now());
    app.diagram_pane_dragging = false;

    crate::tui::mermaid::clear_active_diagrams();
    crate::tui::mermaid::register_active_diagram(0x445, 900, 450, None);
    crate::tui::ui::record_layout_snapshot(
        Rect::new(0, 0, 80, 30),
        Some(Rect::new(80, 0, 40, 30)),
        None,
        None,
    );

    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 80,
        row: 10,
        modifiers: KeyModifiers::empty(),
    });
    assert!(app.diagram_pane_dragging);

    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 72,
        row: 10,
        modifiers: KeyModifiers::empty(),
    });

    assert_eq!(app.diagram_pane_ratio, 40);
    assert_eq!(app.diagram_pane_ratio_from, 40);
    assert_eq!(app.diagram_pane_ratio_target, 40);
    assert!(app.diagram_pane_anim_start.is_none());

    crate::tui::mermaid::clear_active_diagrams();
}

#[test]
fn test_is_scroll_only_key_detects_navigation_inputs() {
    let mut app = create_test_app();

    let (up_code, up_mods) = scroll_up_key(&app);
    assert!(super::input::is_scroll_only_key(&app, up_code, up_mods));

    let (down_code, down_mods) = scroll_down_key(&app);
    assert!(super::input::is_scroll_only_key(&app, down_code, down_mods));

    app.diff_pane_focus = true;
    assert!(super::input::is_scroll_only_key(
        &app,
        KeyCode::Char('j'),
        KeyModifiers::empty()
    ));

    assert!(super::input::is_scroll_only_key(
        &app,
        KeyCode::Char('g'),
        KeyModifiers::ALT
    ));

    assert!(!super::input::is_scroll_only_key(
        &app,
        KeyCode::Char('a'),
        KeyModifiers::empty()
    ));
    assert!(!super::input::is_scroll_only_key(
        &app,
        KeyCode::Enter,
        KeyModifiers::empty()
    ));
}

fn command_cell_fg(
    terminal: &ratatui::Terminal<ratatui::backend::TestBackend>,
    command: &str,
) -> Option<ratatui::style::Color> {
    command_cell_at(terminal, command, 0).map(|cell| cell.fg)
}

/// Find the rendered suggestion row for `command` in the terminal buffer and
/// return the cell at `offset` characters into the command (0 is the leading
/// '/'). Suggestion rows render as `{command}  {description}`, which
/// distinguishes them from the echoed input line that can also contain the
/// typed command text.
fn command_cell_at(
    terminal: &ratatui::Terminal<ratatui::backend::TestBackend>,
    command: &str,
    offset: u16,
) -> Option<ratatui::buffer::Cell> {
    let buf = terminal.backend().buffer();
    for y in 0..buf.area.height {
        let mut line = String::new();
        for x in 0..buf.area.width {
            line.push_str(buf[(x, y)].symbol());
        }
        if let Some(x) = line.find(command) {
            let after = &line[x + command.len()..];
            let is_suggestion_row = after
                .strip_prefix("  ")
                .is_some_and(|desc| desc.starts_with(|c: char| !c.is_whitespace()));
            if is_suggestion_row {
                return Some(buf[(x as u16 + offset, y)].clone());
            }
        }
    }
    None
}

/// Expected fg for characters of a suggestion command that the fuzzy matcher
/// did NOT align with the typed query (dimmed toward black).
fn unmatched_command_fg(base: ratatui::style::Color) -> ratatui::style::Color {
    crate::tui::ui::input_ui::dim_command_color(Some(base))
}

/// Expected fg for characters of a suggestion command that the fuzzy matcher
/// aligned with the typed query (brightened toward white, rendered bold).
fn matched_command_fg(base: ratatui::style::Color) -> ratatui::style::Color {
    crate::tui::ui::input_ui::brighten_command_color(Some(base))
}

/// Assert the fuzzy-match recoloring of one rendered suggestion command:
/// the leading '/' is never part of the highlight, so it must be dimmed,
/// while the first command character (matched by the query) must be the
/// brightened base color and bold.
#[track_caller]
fn assert_command_match_recolored(
    terminal: &ratatui::Terminal<ratatui::backend::TestBackend>,
    command: &str,
    base: ratatui::style::Color,
) {
    let slash = command_cell_at(terminal, command, 0).expect("command not rendered");
    assert_eq!(
        slash.fg,
        unmatched_command_fg(base),
        "leading '/' of {command} should be dimmed base color"
    );
    let matched = command_cell_at(terminal, command, 1).expect("command not rendered");
    assert_eq!(
        matched.fg,
        matched_command_fg(base),
        "matched char of {command} should be brightened base color"
    );
    assert!(
        matched
            .style()
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD),
        "matched char of {command} should be bold"
    );
}

#[test]
fn test_help_topic_suggestions_include_catchup_topics() {
    let app = create_test_app();

    let suggestions = app.get_suggestions_for("/help cat");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/help catchup"));

    let suggestions = app.get_suggestions_for("/help bac");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/help back"));
}

#[test]
fn test_context_command_reports_session_context_snapshot() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.memory_enabled = true;
        app.swarm_enabled = true;
        app.queue_mode = true;
        app.active_skill = Some("debug".to_string());
        app.queued_messages.push("queued follow-up".to_string());
        app.pending_images
            .push(("image/png".to_string(), "abc".to_string()));
        app.side_panel = crate::side_panel::SidePanelSnapshot {
            focused_page_id: Some("goals".to_string()),
            pages: vec![crate::side_panel::SidePanelPage {
                id: "goals".to_string(),
                title: "Goals".to_string(),
                file_path: "".to_string(),
                format: crate::side_panel::SidePanelPageFormat::Markdown,
                source: crate::side_panel::SidePanelPageSource::Managed,
                content: "goal details".to_string(),
                updated_at_ms: 0,
            }],
        };
        crate::todo::save_todos(
            &app.session.id,
            &[crate::todo::TodoItem {
                group: None,
                id: "one".to_string(),
                content: "Inspect context summary".to_string(),
                status: "pending".to_string(),
                priority: "high".to_string(),
                blocked_by: Vec::new(),
                assigned_to: None,
                confidence: Some(crate::todo::ConfidenceState::from_legacy_score(77)),
                completion_confidence: None,
                confidence_history: Vec::new(),
            }],
        )
        .expect("save todos");

        app.input = "/context".to_string();
        app.submit_input();

        let msg = app
            .display_messages()
            .last()
            .expect("missing context report");
        assert_eq!(msg.title.as_deref(), Some("Context"));
        assert!(msg.content.contains("Session Context"));
        assert!(msg.content.contains("Prompt / Context Composition"));
        assert!(msg.content.contains("Compaction"));
        assert!(msg.content.contains("Session State"));
        assert!(msg.content.contains("Todos"));
        assert!(msg.content.contains("Side Panel"));
        assert!(msg.content.contains("Inspect context summary"));
        assert!(msg.content.contains("[pending|high|confidence plausible]"));
        assert!(msg.content.contains("active skill: debug"));
        assert!(msg.content.contains("queue mode: on"));
    });
}

#[test]
fn test_nested_command_suggestions_filter_partial_suffixes() {
    let app = create_test_app();

    let suggestions = app.get_suggestions_for("/config ed");
    assert_eq!(
        suggestions.first().map(|(cmd, _)| cmd.as_str()),
        Some("/config edit")
    );

    let suggestions = app.get_suggestions_for("/alignment ce");
    assert_eq!(
        suggestions.first().map(|(cmd, _)| cmd.as_str()),
        Some("/alignment centered")
    );

    let suggestions = app.get_suggestions_for("/compact mo se");
    assert_eq!(
        suggestions.first().map(|(cmd, _)| cmd.as_str()),
        Some("/compact mode semantic")
    );

    let suggestions = app.get_suggestions_for("/memory st");
    assert_eq!(
        suggestions.first().map(|(cmd, _)| cmd.as_str()),
        Some("/memory status")
    );

    let suggestions = app.get_suggestions_for("/improve st");
    assert!(
        suggestions.iter().any(|(cmd, _)| cmd == "/improve status"),
        "expected /improve status suggestion"
    );

    let suggestions = app.get_suggestions_for("/refactor st");
    assert!(
        suggestions.iter().any(|(cmd, _)| cmd == "/refactor status"),
        "expected /refactor status suggestion"
    );
}

#[test]
fn test_autocomplete_adds_space_for_nested_argument_commands() {
    let mut app = create_test_app();
    app.input = "/goals sh".to_string();
    app.cursor_pos = app.input.len();

    assert!(app.autocomplete());
    assert_eq!(app.input(), "/goals show ");
}

#[test]
fn test_goals_show_suggestions_include_goal_ids() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("repo");
    std::fs::create_dir_all(&project).expect("project dir");
    let prev_home = std::env::var_os("JCODE_HOME");
    crate::env::set_var("JCODE_HOME", temp.path());

    let goal = crate::goal::create_goal(
        crate::goal::GoalCreateInput {
            title: "Ship mobile MVP".to_string(),
            scope: crate::goal::GoalScope::Project,
            ..crate::goal::GoalCreateInput::default()
        },
        Some(&project),
    )
    .expect("create goal");

    let mut app = create_test_app();
    app.session.working_dir = Some(project.display().to_string());

    let suggestions = app.get_suggestions_for("/goals show ");
    assert!(
        suggestions
            .iter()
            .any(|(cmd, _)| cmd == &format!("/goals show {}", goal.id))
    );

    if let Some(prev_home) = prev_home {
        crate::env::set_var("JCODE_HOME", prev_home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
}

fn configure_test_remote_models(app: &mut App) {
    app.is_remote = true;
    app.remote_provider_model = Some("gpt-5.3-codex".to_string());
    app.remote_available_entries = vec!["gpt-5.3-codex".to_string(), "gpt-5.2-codex".to_string()];
}

fn configure_test_remote_models_with_openai_recommendations(app: &mut App) {
    app.is_remote = true;
    app.remote_provider_model = Some("gpt-5.2".to_string());
    app.remote_available_entries = vec![
        "gpt-5.2".to_string(),
        "gpt-5.5".to_string(),
        "gpt-5.4".to_string(),
        "gpt-5.4-pro".to_string(),
        "gpt-5.3-codex-spark".to_string(),
        "gpt-5.3-codex".to_string(),
        "claude-opus-4-8".to_string(),
    ];
    app.remote_model_options = app
        .remote_available_entries
        .iter()
        .filter(|model| model.as_str() != "claude-opus-4-8")
        .cloned()
        .map(|model| crate::provider::ModelRoute {
            model,
            provider: "OpenAI".to_string(),
            api_method: "openai-oauth".to_string(),
            available: true,
            detail: String::new(),
            cheapness: None,
        })
        .collect();
    app.remote_model_options.push(crate::provider::ModelRoute {
        model: "claude-opus-4-8".to_string(),
        provider: "Anthropic".to_string(),
        api_method: "claude-oauth".to_string(),
        available: true,
        detail: String::new(),
        cheapness: None,
    });
    app.remote_model_options.push(crate::provider::ModelRoute {
        model: "claude-opus-4-8".to_string(),
        provider: "Anthropic".to_string(),
        api_method: "claude-api".to_string(),
        available: true,
        detail: String::new(),
        cheapness: None,
    });
}

fn configure_test_remote_openrouter_provider_routes(app: &mut App) {
    app.is_remote = true;
    app.remote_provider_name = Some("openrouter".to_string());
    app.remote_provider_model = Some("anthropic/claude-sonnet-4".to_string());
    app.remote_available_entries = vec!["anthropic/claude-sonnet-4".to_string()];
    app.remote_model_options = vec![
        crate::provider::ModelRoute {
            model: "anthropic/claude-sonnet-4".to_string(),
            provider: "auto".to_string(),
            api_method: "openrouter".to_string(),
            available: true,
            detail: "→ Fireworks".to_string(),
            cheapness: None,
        },
        crate::provider::ModelRoute {
            model: "anthropic/claude-sonnet-4".to_string(),
            provider: "Fireworks".to_string(),
            api_method: "openrouter".to_string(),
            available: true,
            detail: String::new(),
            cheapness: None,
        },
        crate::provider::ModelRoute {
            model: "anthropic/claude-sonnet-4".to_string(),
            provider: "OpenAI".to_string(),
            api_method: "openrouter".to_string(),
            available: true,
            detail: String::new(),
            cheapness: None,
        },
    ];
}

#[test]
fn test_model_picker_preview_filter_parsing() {
    assert_eq!(
        App::model_picker_preview_filter("/model"),
        Some(String::new())
    );
    assert_eq!(
        App::model_picker_preview_filter("/model   gpt-5"),
        Some("gpt-5".to_string())
    );
    assert_eq!(
        App::model_picker_preview_filter("   /models codex"),
        Some("codex".to_string())
    );
    assert_eq!(App::model_picker_preview_filter("/modelx"), None);
    assert_eq!(App::model_picker_preview_filter("hello /model"), None);
}

#[test]
fn test_login_picker_preview_filter_parsing() {
    assert_eq!(
        App::login_picker_preview_filter("/login"),
        Some(String::new())
    );
    assert_eq!(
        App::login_picker_preview_filter("/login   zai"),
        Some("zai".to_string())
    );
    assert_eq!(App::login_picker_preview_filter("/loginx"), None);
    assert_eq!(App::login_picker_preview_filter("hello /login"), None);
}

#[test]
fn test_agents_command_opens_agent_picker() {
    let mut app = create_test_app();
    app.input = "/agents".to_string();

    app.submit_input();

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("/agents should open the agent picker");
    assert!(
        picker
            .entries
            .iter()
            .any(|entry| entry.name == "Code review")
    );
    assert!(picker.entries.iter().any(|entry| matches!(
        entry.action,
        crate::tui::PickerAction::AgentTarget(crate::tui::AgentModelTarget::Swarm)
    )));
    let swarm_entry = picker
        .entries
        .iter()
        .find(|entry| {
            matches!(
                entry.action,
                crate::tui::PickerAction::AgentTarget(crate::tui::AgentModelTarget::Swarm)
            )
        })
        .expect("swarm entry");
    assert!(swarm_entry.options[0].detail.contains("/swarm-prompt"));
}

#[test]
fn test_agents_command_suggestions_include_targets() {
    let app = create_test_app();
    for base in ["/agents", "/config agents", "/config models"] {
        for role in [
            "main", "swarm", "advisor", "review", "judge", "memory", "ambient",
        ] {
            let command = format!("{base} {role}");
            let suggestions = app.get_suggestions_for(&command);
            assert!(
                suggestions.iter().any(|(cmd, description)| {
                    cmd == &command && description.contains("effort")
                }),
                "missing role completion: {command}"
            );
        }
    }
    for (prefix, command) in [
        ("/config ag", "/config agents"),
        ("/config mo", "/config models"),
    ] {
        let suggestions = app.get_suggestions_for(prefix);
        assert!(suggestions.iter().any(|(cmd, _)| cmd == command));
    }
    let help = app.command_help("agents").unwrap();
    assert!(help.contains("Main and advisor affect the current session"));
    assert!(help.contains("saved defaults"));
    assert!(
        app.command_help("config")
            .unwrap()
            .contains("/config models")
    );
    assert!(
        app.command_help("advisor")
            .unwrap()
            .contains("reasoning effort")
    );
}

#[test]
fn test_swarm_prompt_command_is_discoverable_in_suggestions_and_help() {
    let app = create_test_app();
    let suggestions = app.get_suggestions_for("/swarm-pro");
    assert!(
        suggestions
            .iter()
            .any(|(command, _)| command == "/swarm-prompt")
    );

    let help = app
        .command_help("swarm-prompt")
        .expect("/swarm-prompt should have detailed help");
    assert!(help.contains("/swarm-prompt"));
    assert!(help.contains(".jcode/swarm-prompt.md"));
    assert!(help.contains("Restart or reload Jcode"));
}

#[test]
fn test_agents_picker_uses_provider_default_when_inherited_model_is_unknown() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.open_agents_picker();

        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("/agents should open the agent picker");
        let swarm_entry = picker
            .entries
            .iter()
            .find(|entry| {
                matches!(
                    entry.action,
                    crate::tui::PickerAction::AgentTarget(crate::tui::AgentModelTarget::Swarm)
                )
            })
            .expect("swarm entry should exist");

        assert_eq!(swarm_entry.options[0].provider, "provider default");
    });
}

#[test]
fn test_agent_model_picker_inherit_row_uses_provider_default_when_inherited_model_is_unknown() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        configure_test_remote_models(&mut app);
        app.remote_provider_model = Some("unknown".to_string());
        app.open_agent_model_picker(crate::tui::AgentModelTarget::Swarm);

        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("agent model picker should open");
        let inherit_entry = picker.entries.first().expect("inherit row should exist");

        assert_eq!(inherit_entry.name, "inherit (provider default)");
        assert!(matches!(
            inherit_entry.action,
            crate::tui::PickerAction::AgentModelChoice {
                target: crate::tui::AgentModelTarget::Swarm,
                clear_override: true,
            }
        ));
    });
}

include!("command_suggestions.rs");

include!("image_preview.rs");
