#[test]
fn test_remote_typing_resumes_bottom_follow_mode() {
    let mut app = create_test_app();
    app.scroll_offset = 7;
    app.auto_scroll_paused = true;

    app.handle_remote_char_input('x');

    assert_eq!(app.input, "x");
    assert_eq!(app.cursor_pos, 1);
    assert_eq!(app.scroll_offset, 0);
    assert!(
        !app.auto_scroll_paused,
        "typing in remote mode should follow newest content, not pin top"
    );
}
#[test]
fn test_local_typing_resumes_bottom_follow_mode() {
    let mut app = create_test_app();
    app.scroll_offset = 7;
    app.auto_scroll_paused = true;

    app.handle_key(KeyCode::Char('x'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(app.input, "x");
    assert_eq!(app.cursor_pos, 1);
    assert_eq!(app.scroll_offset, 0);
    assert!(
        !app.auto_scroll_paused,
        "local typing should follow newest content just like remote typing"
    );
}
#[test]
fn test_local_typing_snaps_rendered_viewport_to_bottom_in_one_frame() {
    let _lock = scroll_render_test_lock();
    crate::tui::ui::clear_test_render_state_for_tests();

    let (mut app, mut terminal) = create_scroll_test_app(50, 12, 0, 32);
    let _ = render_and_snap(&app, &mut terminal);
    let max_scroll = crate::tui::ui::last_max_scroll();
    assert!(
        max_scroll > 8,
        "expected a long transcript, got {max_scroll}"
    );

    app.auto_scroll_paused = true;
    app.scroll_offset = max_scroll - 8;
    let _ = render_and_snap(&app, &mut terminal);
    assert_eq!(crate::tui::ui::last_resolved_chat_scroll(), max_scroll - 8);

    app.handle_key(KeyCode::Char('x'), KeyModifiers::empty())
        .unwrap();
    let _ = render_and_snap(&app, &mut terminal);

    assert_eq!(
        crate::tui::ui::last_resolved_chat_scroll(),
        crate::tui::ui::last_max_scroll(),
        "typing should explicitly snap to the exact transcript tail, not use content catch-up"
    );
}
#[test]
fn test_remote_shift_slash_preserves_layout_translated_slash() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    rt.block_on(app.handle_remote_key(KeyCode::Char('/'), KeyModifiers::SHIFT, &mut remote))
        .unwrap();

    assert_eq!(app.input(), "/");
    assert_eq!(app.cursor_pos(), 1);
}
#[test]
fn test_remote_key_event_shift_slash_preserves_layout_translated_slash() {
    use crossterm::event::{KeyEvent, KeyEventKind};

    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    rt.block_on(remote::handle_remote_key_event(
        &mut app,
        KeyEvent::new_with_kind(KeyCode::Char('/'), KeyModifiers::SHIFT, KeyEventKind::Press),
        &mut remote,
    ))
    .unwrap();

    assert_eq!(app.input(), "/");
    assert_eq!(app.cursor_pos(), 1);
}
#[test]
fn test_remote_control_alt_symbol_inserts_layout_translated_text() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    rt.block_on(app.handle_remote_key(
        KeyCode::Char('@'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        &mut remote,
    ))
    .unwrap();

    assert_eq!(app.input(), "@");
    assert_eq!(app.cursor_pos(), 1);
}
#[test]
fn test_local_alt_s_toggles_typing_scroll_lock() {
    let mut app = create_test_app();

    app.handle_key(KeyCode::Char('s'), KeyModifiers::ALT)
        .unwrap();
    assert_eq!(
        app.status_notice(),
        Some("Typing scroll lock: ON - typing stays at current chat position".to_string())
    );

    app.handle_key(KeyCode::Char('s'), KeyModifiers::ALT)
        .unwrap();
    assert_eq!(
        app.status_notice(),
        Some("Typing scroll lock: OFF - typing follows chat bottom".to_string())
    );
}
#[test]
fn test_local_alt_m_toggles_side_panel_visibility() {
    let mut app = create_test_app();
    app.side_panel = test_side_panel_snapshot("plan", "Plan");
    app.last_side_panel_focus_id = Some("plan".to_string());

    app.handle_key(KeyCode::Char('m'), KeyModifiers::ALT)
        .unwrap();
    assert_eq!(app.side_panel.focused_page_id, None);
    assert_eq!(app.status_notice(), Some("Side panel: OFF".to_string()));

    app.handle_key(KeyCode::Char('m'), KeyModifiers::ALT)
        .unwrap();
    assert_eq!(app.side_panel.focused_page_id.as_deref(), Some("plan"));
    assert_eq!(app.status_notice(), Some("Side panel: Plan".to_string()));
}
#[test]
fn test_local_alt_m_hidden_side_panel_stays_hidden_across_snapshot_update() {
    let mut app = create_test_app();
    app.side_panel = test_side_panel_snapshot("plan", "Plan");
    app.last_side_panel_focus_id = Some("plan".to_string());

    app.handle_key(KeyCode::Char('m'), KeyModifiers::ALT)
        .unwrap();
    assert_eq!(app.side_panel.focused_page_id, None);

    app.set_side_panel_snapshot(test_side_panel_snapshot("plan", "Updated plan"));
    assert_eq!(app.side_panel.focused_page_id, None);
    assert_eq!(app.side_panel.pages[0].title, "Updated plan");

    app.handle_key(KeyCode::Char('m'), KeyModifiers::ALT)
        .unwrap();
    assert_eq!(app.side_panel.focused_page_id.as_deref(), Some("plan"));
    assert_eq!(
        app.status_notice(),
        Some("Side panel: Updated plan".to_string())
    );
}
#[test]
fn test_local_alt_m_falls_back_to_diagram_pane_when_side_panel_is_empty() {
    let mut app = create_test_app();
    app.side_panel = crate::side_panel::SidePanelSnapshot::default();
    app.diagram_pane_enabled = true;

    app.handle_key(KeyCode::Char('m'), KeyModifiers::ALT)
        .unwrap();

    assert!(!app.diagram_pane_enabled);
    assert_eq!(app.status_notice(), Some("Diagram pane: OFF".to_string()));
}
#[test]
fn test_images_do_not_drive_side_panel_visibility() {
    // Images now render inline in the transcript flow, so they must not flip the
    // side panel on, arm an auto-hide timer, or otherwise behave like the old
    // pinned-image side pane.
    let mut app = create_test_app();
    app.is_remote = true;
    app.side_panel = crate::side_panel::SidePanelSnapshot::default();
    app.remote_side_pane_images
        .push(crate::session::RenderedImage {
            media_type: "image/png".to_string(),
            data: "image-data".to_string(),
            label: Some("preview.png".to_string()),
            source: crate::session::RenderedImageSource::UserInput,
            anchor: None,
        });

    // Auto-hide bookkeeping is now a no-op for images.
    assert!(!app.update_pinned_images_auto_hide());
    assert!(app.pinned_images_auto_hide_deadline.is_none());
    assert!(!app.side_panel_user_hidden);
}
#[test]
fn test_remote_alt_m_toggles_side_panel_visibility() {
    let mut app = create_test_app();
    app.side_panel = test_side_panel_snapshot("plan", "Plan");
    app.last_side_panel_focus_id = Some("plan".to_string());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    rt.block_on(app.handle_remote_key(KeyCode::Char('m'), KeyModifiers::ALT, &mut remote))
        .unwrap();
    assert_eq!(app.side_panel.focused_page_id, None);
    assert_eq!(app.status_notice(), Some("Side panel: OFF".to_string()));

    rt.block_on(app.handle_remote_key(KeyCode::Char('m'), KeyModifiers::ALT, &mut remote))
        .unwrap();
    assert_eq!(app.side_panel.focused_page_id.as_deref(), Some("plan"));
    assert_eq!(app.status_notice(), Some("Side panel: Plan".to_string()));
}
#[test]
fn test_remote_alt_y_toggles_copy_selection_instead_of_typing() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    rt.block_on(app.handle_remote_key(KeyCode::Char('y'), KeyModifiers::ALT, &mut remote))
        .unwrap();

    assert!(app.copy_selection_mode);
    assert!(app.input.is_empty(), "Alt+Y must not insert text");
}
#[test]
fn test_remote_alt_i_toggles_info_widget_instead_of_typing() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();
    let initially_enabled = crate::tui::info_widget::is_enabled();

    rt.block_on(app.handle_remote_key(KeyCode::Char('i'), KeyModifiers::ALT, &mut remote))
        .unwrap();

    assert_ne!(crate::tui::info_widget::is_enabled(), initially_enabled);
    assert!(app.input.is_empty(), "Alt+I must not insert text");
    crate::tui::info_widget::toggle_enabled();
}
#[test]
fn test_remote_typing_scroll_lock_preserves_scroll_position() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    app.scroll_offset = 7;
    app.auto_scroll_paused = true;

    rt.block_on(app.handle_remote_key(KeyCode::Char('s'), KeyModifiers::ALT, &mut remote))
        .unwrap();
    app.handle_remote_char_input('x');

    assert_eq!(app.input, "x");
    assert_eq!(app.cursor_pos, 1);
    assert_eq!(app.scroll_offset, 7);
    assert!(
        app.auto_scroll_paused,
        "typing scroll lock should preserve paused scroll state"
    );
}
#[test]
fn test_local_typing_scroll_lock_preserves_scroll_position() {
    let mut app = create_test_app();
    app.scroll_offset = 7;
    app.auto_scroll_paused = true;

    app.handle_key(KeyCode::Char('s'), KeyModifiers::ALT)
        .unwrap();
    app.handle_key(KeyCode::Char('x'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(app.input, "x");
    assert_eq!(app.cursor_pos, 1);
    assert_eq!(app.scroll_offset, 7);
    assert!(
        app.auto_scroll_paused,
        "typing scroll lock should preserve local paused scroll state"
    );
}
#[test]
fn test_remote_typing_scroll_lock_can_be_toggled_back_off() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    app.scroll_offset = 7;
    app.auto_scroll_paused = true;

    rt.block_on(app.handle_remote_key(KeyCode::Char('s'), KeyModifiers::ALT, &mut remote))
        .unwrap();
    rt.block_on(app.handle_remote_key(KeyCode::Char('s'), KeyModifiers::ALT, &mut remote))
        .unwrap();
    app.handle_remote_char_input('x');

    assert_eq!(app.scroll_offset, 0);
    assert!(
        !app.auto_scroll_paused,
        "typing should resume following chat bottom after disabling the lock"
    );
}
