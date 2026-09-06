#[test]
fn test_copy_selection_reconstructs_wrapped_chat_lines_without_hard_wraps() {
    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    app.display_messages = vec![DisplayMessage {
        role: "assistant".to_string(),
        content: "same physical device: i2c-ELAN900C:00 same vendor/product family: 04F3:4216"
            .to_string(),
        tool_calls: vec![],
        duration_secs: None,
        title: None,
        tool_data: None,
    }];
    app.bump_display_messages_version();

    let backend = ratatui::backend::TestBackend::new(36, 20);
    let mut terminal = ratatui::Terminal::new(backend).expect("failed to create test terminal");

    render_and_snap(&app, &mut terminal);
    let (visible_start, visible_end) =
        crate::tui::ui::copy_viewport_visible_range().expect("visible copy range");

    let visible_lines: Vec<(usize, String)> = (visible_start..visible_end)
        .filter_map(|abs_line| {
            let text = crate::tui::ui::copy_viewport_line_text(abs_line)?;
            (!text.is_empty()).then_some((abs_line, text))
        })
        .collect();
    let (first_idx, _first_text) = visible_lines
        .iter()
        .find(|(_, text)| text.contains("i2c-ELAN900C:00"))
        .expect("expected wrapped line containing device path");
    let (second_idx, second_text) = visible_lines
        .iter()
        .find(|(idx, _)| *idx == *first_idx + 1)
        .expect("expected adjacent wrapped continuation line");

    app.copy_selection_anchor = Some(crate::tui::CopySelectionPoint {
        pane: crate::tui::CopySelectionPane::Chat,
        abs_line: *first_idx,
        column: 0,
    });
    app.copy_selection_cursor = Some(crate::tui::CopySelectionPoint {
        pane: crate::tui::CopySelectionPane::Chat,
        abs_line: *second_idx,
        column: unicode_width::UnicodeWidthStr::width(second_text.as_str()),
    });

    let selected = app
        .current_copy_selection_text()
        .expect("expected wrapped selection text");
    assert!(
        !selected.contains('\n'),
        "wrapped chat copy should not include a hard newline: {selected:?}"
    );
    assert!(
        selected.contains("i2c-ELAN900C:00"),
        "selection should include the device path: {selected:?}"
    );
    assert!(
        selected.contains("same vendor/product family"),
        "selection should preserve the natural space across wrapped lines: {selected:?}"
    );
}
#[test]
fn test_copy_selection_centered_list_keeps_logical_list_text() {
    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    app.set_centered(true);
    app.display_messages = vec![DisplayMessage {
        role: "assistant".to_string(),
        content: concat!(
            "A goal should support\n\n",
            "1. Create a goal\n",
            "\n",
            "- title\n",
            "- description / \"why this matters\"\n",
            "- success criteria\n",
        )
        .to_string(),
        tool_calls: vec![],
        duration_secs: None,
        title: None,
        tool_data: None,
    }];
    app.bump_display_messages_version();

    let backend = ratatui::backend::TestBackend::new(28, 20);
    let mut terminal = ratatui::Terminal::new(backend).expect("failed to create test terminal");

    render_and_snap(&app, &mut terminal);
    let (visible_start, visible_end) =
        crate::tui::ui::copy_viewport_visible_range().expect("visible copy range");
    let visible_lines: Vec<(usize, String)> = (visible_start..visible_end)
        .filter_map(|abs_line| {
            let text = crate::tui::ui::copy_viewport_line_text(abs_line)?;
            (!text.is_empty()).then_some((abs_line, text))
        })
        .collect();

    let (start_idx, _) = visible_lines
        .iter()
        .find(|(_, text)| text.contains("1. Create a goal"))
        .expect("numbered list line");
    let (end_idx, end_text) = visible_lines
        .iter()
        .rev()
        .find(|(_, text)| text.contains("success criteria") || text.contains("matters"))
        .expect("last list line");

    app.copy_selection_anchor = Some(crate::tui::CopySelectionPoint {
        pane: crate::tui::CopySelectionPane::Chat,
        abs_line: *start_idx,
        column: 0,
    });
    app.copy_selection_cursor = Some(crate::tui::CopySelectionPoint {
        pane: crate::tui::CopySelectionPane::Chat,
        abs_line: *end_idx,
        column: unicode_width::UnicodeWidthStr::width(end_text.as_str()),
    });

    let selected = app
        .current_copy_selection_text()
        .expect("expected selected list text");

    assert!(
        selected.contains("1. Create a goal"),
        "numbered list item should be copied without centered padding: {selected:?}"
    );
    assert!(
        selected.contains("• title"),
        "bullet item should be copied without centered padding: {selected:?}"
    );
    assert!(
        selected.contains("why this matters"),
        "wrapped bullet item should copy logical text: {selected:?}"
    );
}
#[test]
fn test_copy_selection_mouse_drag_extracts_expected_multiline_range() {
    let _render_lock = scroll_render_test_lock();
    let (mut app, mut terminal) = create_copy_test_app();

    render_and_snap(&app, &mut terminal);
    app.handle_key(KeyCode::Char('y'), KeyModifiers::ALT)
        .unwrap();

    let layout = crate::tui::ui::last_layout_snapshot().expect("layout snapshot");
    let (visible_start, visible_end) =
        crate::tui::ui::copy_viewport_visible_range().expect("visible copy range");

    let mut fn_line = None;
    let mut print_line = None;
    for abs_line in visible_start..visible_end {
        let text = crate::tui::ui::copy_viewport_line_text(abs_line).unwrap_or_default();
        if text.contains("fn main() {") {
            fn_line = Some((abs_line, text.clone()));
        }
        if text.contains("println!(\"hello\");") {
            print_line = Some((abs_line, text));
        }
    }

    let (fn_line_idx, fn_text) = fn_line.expect("fn line");
    let (print_line_idx, print_text) = print_line.expect("println line");
    let fn_byte = fn_text.find("fn main() {").expect("fn column");
    let fn_col = unicode_width::UnicodeWidthStr::width(&fn_text[..fn_byte]) as u16;
    let _print_end_col = (print_text.find(");").expect("print end") + 2) as u16;

    let base_y = layout.messages_area.y;
    let start_row = base_y + (fn_line_idx - visible_start) as u16;
    let end_row = base_y + (print_line_idx - visible_start) as u16;

    let start_x = (layout.messages_area.x..layout.messages_area.x + layout.messages_area.width)
        .find(|&column| {
            crate::tui::ui::copy_viewport_point_from_screen(column, start_row)
                .map(|point| point.abs_line == fn_line_idx && point.column == fn_col as usize)
                .unwrap_or(false)
        })
        .expect("screen x for selection start");

    let end_x = (layout.messages_area.x..layout.messages_area.x + layout.messages_area.width)
        .filter_map(|column| {
            crate::tui::ui::copy_viewport_point_from_screen(column, end_row)
                .filter(|point| point.abs_line == print_line_idx)
                .map(|point| (column, point.column))
        })
        .max_by_key(|(_, mapped_col)| *mapped_col)
        .map(|(column, _)| column)
        .expect("screen x for selection end");

    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: start_x,
        row: start_row,
        modifiers: KeyModifiers::empty(),
    });
    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: end_x,
        row: end_row,
        modifiers: KeyModifiers::empty(),
    });

    let selected = app
        .current_copy_selection_text()
        .expect("expected multiline selection");
    let range = app.normalized_copy_selection().expect("normalized range");
    assert_eq!(range.start.abs_line, fn_line_idx);
    assert_eq!(range.end.abs_line, print_line_idx);
    assert!(
        selected.contains("fn main() {"),
        "selection missing fn line: {selected}"
    );
    assert!(
        selected.contains("println!(\"hello\");"),
        "selection missing println line: {selected}"
    );
    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: end_x,
        row: end_row,
        modifiers: KeyModifiers::empty(),
    });
    assert!(app.copy_selection_mode);
    assert!(!app.copy_selection_dragging);
}
#[test]
fn test_copy_selection_mouse_click_does_not_enter_mode() {
    let _render_lock = scroll_render_test_lock();
    let (mut app, mut terminal) = create_copy_test_app();

    render_and_snap(&app, &mut terminal);

    let layout = crate::tui::ui::last_layout_snapshot().expect("layout snapshot");
    let (visible_start, visible_end) =
        crate::tui::ui::copy_viewport_visible_range().expect("visible copy range");

    let target = (visible_start..visible_end)
        .find_map(|abs_line| {
            let text = crate::tui::ui::copy_viewport_line_text(abs_line)?;
            let byte = text.find("println!(\"hello\");")?;
            let col = unicode_width::UnicodeWidthStr::width(&text[..byte]) as u16;
            Some((abs_line, col))
        })
        .expect("println line");

    let row = layout.messages_area.y + (target.0 - visible_start) as u16;
    let col = (layout.messages_area.x..layout.messages_area.x + layout.messages_area.width)
        .find(|&column| {
            crate::tui::ui::copy_viewport_point_from_screen(column, row)
                .map(|point| point.abs_line == target.0 && point.column == target.1 as usize)
                .unwrap_or(false)
        })
        .expect("screen x for println");

    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: KeyModifiers::empty(),
    });
    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: col,
        row,
        modifiers: KeyModifiers::empty(),
    });

    assert!(!app.copy_selection_mode);
    assert!(app.copy_selection_anchor.is_none());
    assert!(app.copy_selection_cursor.is_none());
}
#[test]
fn test_copy_selection_mouse_drag_auto_copies_and_keeps_highlight() {
    let _render_lock = scroll_render_test_lock();
    let (mut app, mut terminal) = create_copy_test_app();
    let copied = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let copied_for_closure = copied.clone();

    render_and_snap(&app, &mut terminal);

    let layout = crate::tui::ui::last_layout_snapshot().expect("layout snapshot");
    let (visible_start, visible_end) =
        crate::tui::ui::copy_viewport_visible_range().expect("visible copy range");

    let mut fn_line = None;
    let mut print_line = None;
    for abs_line in visible_start..visible_end {
        let text = crate::tui::ui::copy_viewport_line_text(abs_line).unwrap_or_default();
        if text.contains("fn main() {") {
            fn_line = Some((abs_line, text.clone()));
        }
        if text.contains("println!(\"hello\");") {
            print_line = Some((abs_line, text));
        }
    }

    let (fn_line_idx, fn_text) = fn_line.expect("fn line");
    let (print_line_idx, _print_text) = print_line.expect("println line");
    let fn_byte = fn_text.find("fn main() {").expect("fn column");
    let fn_col = unicode_width::UnicodeWidthStr::width(&fn_text[..fn_byte]) as u16;

    let base_y = layout.messages_area.y;
    let start_row = base_y + (fn_line_idx - visible_start) as u16;
    let end_row = base_y + (print_line_idx - visible_start) as u16;

    let start_x = (layout.messages_area.x..layout.messages_area.x + layout.messages_area.width)
        .find(|&column| {
            crate::tui::ui::copy_viewport_point_from_screen(column, start_row)
                .map(|point| point.abs_line == fn_line_idx && point.column == fn_col as usize)
                .unwrap_or(false)
        })
        .expect("screen x for selection start");

    let end_x = (layout.messages_area.x..layout.messages_area.x + layout.messages_area.width)
        .filter_map(|column| {
            crate::tui::ui::copy_viewport_point_from_screen(column, end_row)
                .filter(|point| point.abs_line == print_line_idx)
                .map(|point| (column, point.column))
        })
        .max_by_key(|(_, mapped_col)| *mapped_col)
        .map(|(column, _)| column)
        .expect("screen x for selection end");

    app.handle_copy_selection_mouse_with(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: start_x,
            row: start_row,
            modifiers: KeyModifiers::empty(),
        },
        |_| true,
    );
    app.handle_copy_selection_mouse_with(
        MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: end_x,
            row: end_row,
            modifiers: KeyModifiers::empty(),
        },
        |_| true,
    );
    app.handle_copy_selection_mouse_with(
        MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: end_x,
            row: end_row,
            modifiers: KeyModifiers::empty(),
        },
        |text| {
            *copied_for_closure.lock().unwrap() = text.to_string();
            true
        },
    );

    assert!(!app.copy_selection_mode);
    assert!(app.copy_selection_anchor.is_some());
    assert!(app.copy_selection_cursor.is_some());
    assert!(copied.lock().unwrap().contains("println!(\"hello\");"));
    assert_eq!(
        app.status_notice(),
        Some("Copied selection · highlight remains visible".to_string())
    );
}
