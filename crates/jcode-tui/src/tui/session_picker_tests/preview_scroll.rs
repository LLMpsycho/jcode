#[test]
fn test_preview_pane_shows_scrollbar_when_overflowing() {
    let session = make_session_with_many_turns("preview_scroll", 60);
    let mut picker = SessionPicker::new(vec![session]);
    picker.focus = PaneFocus::Preview;

    // Small height so the long preview overflows and needs a scrollbar.
    let text = buffer_text(&mut picker, 100, 16);
    assert!(
        contains_scrollbar_glyph(&text),
        "preview scrollbar glyph should render when content overflows:\n{text}"
    );
}
#[test]
fn test_session_list_shows_scrollbar_when_overflowing() {
    // Many sessions so the left list overflows a short viewport.
    let sessions: Vec<SessionInfo> = (0..40)
        .map(|i| {
            make_session(
                &format!("list_scroll_{i}"),
                &format!("s{i}"),
                false,
                SessionStatus::Closed,
            )
        })
        .collect();
    let mut picker = SessionPicker::new(sessions);
    picker.focus = PaneFocus::Sessions;

    let text = buffer_text(&mut picker, 100, 16);
    assert!(
        contains_scrollbar_glyph(&text),
        "session list scrollbar glyph should render when list overflows:\n{text}"
    );
}
#[test]
fn test_preview_sticky_prompt_header_appears_after_scrolling() {
    let session = make_session_with_many_turns("sticky_header", 60);
    let mut picker = SessionPicker::new(vec![session]);
    picker.focus = PaneFocus::Preview;

    // First render auto-scrolls to the bottom; the topmost prompts are off-screen,
    // so a dimmed "N› ..." sticky header should pin a prior prompt at the top of
    // the preview's content area.
    let w = 100u16;
    let h = 16u16;
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| picker.render(frame))
        .expect("render picker");
    let buffer = terminal.backend().buffer().clone();

    // The preview pane occupies the right 60% of the width; its inner content
    // starts just inside the rounded border. Read the first inner content row and
    // confirm it carries the "N›" sticky-header marker.
    let preview_inner_x = (w as f32 * 0.40) as u16 + 1;
    let header_row: String = (preview_inner_x..w.saturating_sub(1))
        .map(|x| buffer[(x, 1)].symbol())
        .collect();
    assert!(
        header_row.contains('›'),
        "sticky prompt header should pin a numbered prompt at the top of the preview:\n\
         row={header_row:?}"
    );
    // The header marker is a prompt number followed by the chevron.
    assert!(
        header_row
            .trim_start()
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit()),
        "sticky header should begin with a prompt number:\nrow={header_row:?}"
    );
}
#[test]
fn test_preview_sticky_prompt_header_survives_async_preview_load() {
    // Regression: when the selected session's transcript is still loading on a
    // background thread, the first render only shows a "Loading…" placeholder
    // (max_scroll == 0). The auto-scroll flag must NOT be consumed on that
    // placeholder frame; otherwise the populated transcript stays pinned at the
    // top and the sticky "previous prompt" header never appears (it only renders
    // when scrolled past a prompt). This reproduces the intermittent "/resume
    // sometimes doesn't show your last prompt at the top" bug.
    let mut session = make_session_with_many_turns("async_sticky", 60);
    let full_preview = std::mem::take(&mut session.messages_preview);
    session.first_user_prompt = full_preview.first().map(|m| m.content.clone());

    let mut picker = SessionPicker::new(vec![session.clone()]);
    picker.focus = PaneFocus::Preview;

    // Simulate an in-flight background load for the selected session: empty
    // preview + a pending load whose id matches. Keep the sender alive so the
    // receiver reports `Empty` (still loading) rather than `Disconnected`.
    let (tx, rx) = std::sync::mpsc::channel::<Option<Vec<PreviewMessage>>>();
    picker.pending_preview_load = Some(PendingSessionPreviewLoad {
        session_id: "async_sticky".to_string(),
        receiver: rx,
    });

    let w = 100u16;
    let h = 16u16;
    let render = |picker: &mut SessionPicker| {
        let backend = ratatui::backend::TestBackend::new(w, h);
        let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| picker.render(frame))
            .expect("render picker");
        terminal.backend().buffer().clone()
    };

    // First frame: transcript still loading -> placeholder shown, auto-scroll
    // must remain armed.
    let _ = render(&mut picker);
    assert!(
        picker.auto_scroll_preview,
        "auto-scroll should stay armed while the preview is still loading"
    );

    // The background load completes: deliver the real transcript exactly the way
    // `poll_preview_load` would (drop the channel + populate the preview).
    drop(tx);
    picker.pending_preview_load = None;
    picker.apply_session_preview("async_sticky", full_preview);

    // Second frame: now that content is present we snap to the bottom and the
    // top prompts scroll off-screen, so the sticky header should pin a prompt.
    let buffer = render(&mut picker);
    assert!(
        picker.scroll_offset > 0,
        "preview should auto-scroll to the bottom once content loads, got {}",
        picker.scroll_offset
    );

    let preview_inner_x = (w as f32 * 0.40) as u16 + 1;
    let header_row: String = (preview_inner_x..w.saturating_sub(1))
        .map(|x| buffer[(x, 1)].symbol())
        .collect();
    assert!(
        header_row.contains('›'),
        "sticky prompt header should appear after an async preview load:\n\
         row={header_row:?}"
    );
    assert!(
        header_row
            .trim_start()
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit()),
        "sticky header should begin with a prompt number:\nrow={header_row:?}"
    );
}
#[test]
fn preview_render_cache_is_reused_across_scroll_and_rebuilt_on_selection_change() {
    // The preview pane caches its fully-wrapped content keyed by a content hash
    // and pane geometry, so scrolling reuses the cache instead of re-rendering
    // and re-wrapping every line. Navigating to another session must invalidate
    // it (different content hash).
    let a = make_session_with_many_turns("cache_a", 60);
    let b = make_session_with_many_turns("cache_b", 60);
    let mut picker = SessionPicker::new(vec![a, b]);
    picker.focus = PaneFocus::Preview;

    let w = 100u16;
    let h = 16u16;
    let render = |picker: &mut SessionPicker| {
        let backend = ratatui::backend::TestBackend::new(w, h);
        let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| picker.render(frame))
            .expect("render picker");
    };

    // First render builds the cache for the selected session.
    render(&mut picker);
    let key_after_build = picker
        .preview_cache
        .as_ref()
        .map(|c| c.key.clone())
        .expect("preview cache built on first render");
    let wrapped_len = picker
        .preview_cache
        .as_ref()
        .map(|c| c.wrapped_lines.len())
        .unwrap();
    assert!(wrapped_len > h as usize, "preview should overflow viewport");

    // Scrolling several times must not change the cache key (content unchanged):
    // the cache is reused and only the scroll offset + visible slice move.
    for _ in 0..5 {
        picker.scroll_preview_up(1);
        render(&mut picker);
        let key_now = picker
            .preview_cache
            .as_ref()
            .map(|c| c.key.clone())
            .unwrap();
        assert!(
            key_now == key_after_build,
            "scrolling must reuse the cached wrapped preview"
        );
    }

    // Navigating to a different session changes the content hash, so the cache
    // is rebuilt for the new selection.
    picker.next();
    render(&mut picker);
    let key_after_nav = picker
        .preview_cache
        .as_ref()
        .map(|c| c.key.clone())
        .unwrap();
    assert!(
        key_after_nav != key_after_build,
        "selecting a different session must invalidate the preview cache"
    );
}
#[test]
fn preview_visible_slice_matches_scroll_position() {
    // The renderer materializes only the visible window of wrapped lines. Confirm
    // that scrolling actually changes what is drawn (i.e. the slice tracks the
    // scroll offset rather than always showing the bottom).
    let session = make_session_with_many_turns("slice", 60);
    let mut picker = SessionPicker::new(vec![session]);
    picker.focus = PaneFocus::Preview;

    let w = 100u16;
    let h = 16u16;
    let render_text = |picker: &mut SessionPicker| -> String { buffer_text(picker, w, h) };

    // First render auto-scrolls to the bottom.
    let bottom = render_text(&mut picker);
    let bottom_scroll = picker.scroll_offset;
    assert!(bottom_scroll > 0, "long preview should be scrolled down");

    // Scroll to the very top; the rendered content must differ from the bottom.
    picker.scroll_preview_up(bottom_scroll);
    let top = render_text(&mut picker);
    assert_eq!(picker.scroll_offset, 0);
    assert_ne!(
        top, bottom,
        "scrolling to the top should render different content than the bottom"
    );
}
