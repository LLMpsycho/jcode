/// End-to-end: a real left-click on an inline image's label line maps the
/// screen point back through a recorded `ChatFrame` snapshot to the image id and
/// cycles its expand level. This exercises the full click path
/// (`handle_mouse_event` -> `try_cycle_image_expand_at` ->
/// `inline_image_expand_target_from_screen` -> `cycle_image_expand`), not just
/// the isolated helpers.
#[test]
fn test_click_on_inline_image_label_line_cycles_level() {
    use crate::tui::ui::inline_image_ui::{
        AllFit, ImageExpandLevel, InlineImageItem, build_section,
    };
    use jcode_tui_messages::PreparedChatFrame;

    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();

    const IMAGE_ID: u64 = 0xFEED;
    let chat_width: u16 = 80;

    // Build a real inline-image section: a `shot.png … hide` label line
    // followed by Fit-rendered placeholder rows with a scanned `image_regions`
    // entry.
    let items = vec![InlineImageItem {
        id: IMAGE_ID,
        width: 600,
        height: 400,
        label: "shot.png".to_string(),
        uses_text_fallback: true,
    }];
    let section = build_section(&items, chat_width, 40, false, true, &AllFit);

    // Locate the label line (the one carrying the image label); the whole line
    // is the click target now that the expand badge is gone.
    let label_line = section
        .wrapped_plain_lines
        .iter()
        .position(|line| line.contains("shot.png"))
        .expect("section should contain the image label line");

    // Even with the terminal fallback note attached below the image, the Fit
    // region must remain exactly one line below the label. This adjacency is how
    // `inline_image_id_for_label_line` maps a click back to the image.
    assert!(
        section
            .image_regions
            .iter()
            .any(|r| r.hash == IMAGE_ID && r.abs_line_idx == label_line + 1),
        "expected a Fit image region anchored under the label line"
    );

    let prepared =
        std::sync::Arc::new(PreparedChatFrame::from_single(std::sync::Arc::new(section)));
    let visible_end = prepared.wrapped_plain_line_count();
    let content_area = Rect::new(0, 0, chat_width, visible_end as u16 + 1);

    crate::tui::ui::clear_copy_viewport_snapshot();
    crate::tui::ui::record_copy_viewport_frame_snapshot_for_test(
        prepared,
        0,
        visible_end,
        content_area,
        &vec![0u16; visible_end],
    );

    assert_eq!(
        app.image_expand_level(IMAGE_ID),
        ImageExpandLevel::Fit,
        "image should start at Fit"
    );

    // Click the label line (button up is what fires the cycle).
    let handled = app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: content_area.x + 2,
        row: content_area.y + label_line as u16,
        modifiers: KeyModifiers::empty(),
    });
    assert!(!handled, "handled click should request an immediate redraw");
    assert_eq!(
        app.image_expand_level(IMAGE_ID),
        ImageExpandLevel::Large,
        "first label click should expand Fit -> Large"
    );
    assert_eq!(app.status_notice(), Some("Image size: large".to_string()));

    // Large and Full have identical geometry for this landscape image, so the
    // redundant Full state is skipped and the next click returns to Fit.
    let click_label = |app: &mut App| {
        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: content_area.x + 2,
            row: content_area.y + label_line as u16,
            modifiers: KeyModifiers::empty(),
        });
    };
    click_label(&mut app);
    assert_eq!(
        app.image_expand_level(IMAGE_ID),
        ImageExpandLevel::Fit,
        "second click should skip duplicate Full geometry and return to Fit"
    );
}
/// Kitty reports mouse motion at pixel granularity, so a physically plain
/// click usually arrives as Down -> Drag(same cell) -> Up. The same-cell Drag
/// must NOT start a selection drag; the release must still fall through to the
/// label-line click handler. Regression test for "click does nothing on
/// kitty".
#[test]
fn test_kitty_jitter_click_on_image_label_still_cycles_level() {
    use crate::tui::ui::inline_image_ui::{
        AllFit, ImageExpandLevel, InlineImageItem, build_section,
    };
    use jcode_tui_messages::PreparedChatFrame;

    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();

    const IMAGE_ID: u64 = 0xF00D;
    let chat_width: u16 = 80;
    let items = vec![InlineImageItem {
        id: IMAGE_ID,
        width: 600,
        height: 400,
        label: "shot.png".to_string(),
        uses_text_fallback: false,
    }];
    let section = build_section(&items, chat_width, 40, false, true, &AllFit);
    let label_line = section
        .wrapped_plain_lines
        .iter()
        .position(|line| line.contains("shot.png"))
        .expect("section should contain the image label line");
    let badge_col: u16 = 2;

    let prepared =
        std::sync::Arc::new(PreparedChatFrame::from_single(std::sync::Arc::new(section)));
    let visible_end = prepared.wrapped_plain_line_count();
    let content_area = Rect::new(0, 0, chat_width, visible_end as u16 + 1);

    crate::tui::ui::clear_copy_viewport_snapshot();
    crate::tui::ui::record_copy_viewport_frame_snapshot_for_test(
        prepared,
        0,
        visible_end,
        content_area,
        &vec![0u16; visible_end],
    );

    let (col, row) = (
        content_area.x + badge_col,
        content_area.y + label_line as u16,
    );
    let inject = |app: &mut App, kind: MouseEventKind| {
        app.handle_mouse_event(MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::empty(),
        });
    };

    // Down, same-cell Drag (kitty pixel jitter), Up: must count as a click.
    inject(&mut app, MouseEventKind::Down(MouseButton::Left));
    inject(&mut app, MouseEventKind::Drag(MouseButton::Left));
    inject(&mut app, MouseEventKind::Up(MouseButton::Left));

    assert_eq!(
        app.image_expand_level(IMAGE_ID),
        ImageExpandLevel::Large,
        "jitter click (down + same-cell drag + up) must still cycle the badge"
    );

    // A real drag to a DIFFERENT cell must still start a selection, not click.
    inject(&mut app, MouseEventKind::Down(MouseButton::Left));
    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: col.saturating_sub(4),
        row,
        modifiers: KeyModifiers::empty(),
    });
    inject(&mut app, MouseEventKind::Up(MouseButton::Left));
    assert_eq!(
        app.image_expand_level(IMAGE_ID),
        ImageExpandLevel::Large,
        "a real drag ending on the badge must not fire the click handler"
    );
}
/// FULL end-to-end reproduction of the user's "clicking the image does
/// nothing" report. Unlike `test_click_on_inline_image_label_line_cycles_level`
/// (which records a synthetic `ChatFrame` snapshot directly), this drives the
/// *real* draw: a local App whose session carries a `read`-tool result image,
/// anchored into the transcript body, rendered through `terminal.draw()`, which
/// is what records the live copy-viewport snapshot. We then locate the rendered
/// image label line in the actual frame buffer and inject a real left click,
/// asserting the image size cycles. This exercises the body-anchored image path
/// (`render_images` -> `resolve_anchored_items` -> `anchored_image_lines`), the
/// path actually used in production, not the isolated `build_section` helper.
#[test]
fn test_real_draw_click_on_body_anchored_image_label_cycles_level() {
    use crate::message::{ContentBlock, Role};
    use crate::tui::ui::inline_image_ui::ImageExpandLevel;

    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    assert!(!app.is_remote, "repro must use the local image render path");

    const TOOL_ID: &str = "read-shot-1";

    // Build a real transcript: user asks, assistant calls `read`, tool result
    // carries the screenshot image. This is exactly what produces a
    // body-anchored inline image with a `RenderedImageAnchor::ToolCall`.
    app.session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "read the screenshot".to_string(),
            cache_control: None,
        }],
    );
    app.session.add_message(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: TOOL_ID.to_string(),
            name: "read".to_string(),
            input: serde_json::json!({"file_path": "shot.png"}),
            thought_signature: None,
        }],
    );
    app.session.add_message(
        Role::User,
        vec![
            ContentBlock::ToolResult {
                tool_use_id: TOOL_ID.to_string(),
                content: "read image".to_string(),
                is_error: None,
            },
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: REPRO_TINY_PNG_B64.to_string(),
            },
        ],
    );

    // Mirror the session into the display transcript the body renderer walks.
    app.display_messages = vec![
        DisplayMessage::user("read the screenshot"),
        DisplayMessage::tool(
            "read shot.png",
            crate::message::ToolCall {
                id: TOOL_ID.to_string(),
                name: "read".to_string(),
                input: serde_json::json!({"file_path": "shot.png"}),
                intent: None,
                thought_signature: None,
            },
        ),
    ];
    app.bump_display_messages_version();
    app.invalidate_side_pane_images_signature();
    app.pin_images = true;
    app.inline_images_visible = true;
    app.scroll_offset = 0;
    app.auto_scroll_paused = false;
    app.is_processing = false;
    app.status = ProcessingStatus::Idle;
    app.session.short_name = Some("test".to_string());

    // Sanity: the local render path must actually surface the anchored image.
    let images = <App as crate::tui::TuiState>::side_pane_images(&app);
    assert_eq!(
        images.len(),
        1,
        "session should render exactly one anchored tool image"
    );
    let image_id = {
        let img = &images[0];
        crate::tui::mermaid::inline_image_dims(&img.media_type, &img.data)
            .expect("tiny png should decode")
            .0
    };

    let backend = ratatui::backend::TestBackend::new(80, 40);
    let mut terminal = ratatui::Terminal::new(backend).expect("failed to create test terminal");

    // REAL draw: this records the live copy-viewport snapshot used by clicks.
    let rendered = render_and_snap(&app, &mut terminal);
    assert!(
        rendered.contains("shot.png"),
        "image label line must render in the live frame, got:\n{rendered}"
    );

    // Find the label line in the actual buffer: scan rows for the row carrying
    // the image label, then click a cell inside the label text.
    let buf = terminal.backend().buffer();
    let area = *buf.area();
    let mut badge: Option<(u16, u16)> = None;
    'rows: for row in 0..area.height {
        let mut line = String::new();
        for col in 0..area.width {
            line.push_str(buf[(col, row)].symbol());
        }
        // The transcript also shows the tool-call row ("read shot.png"); the
        // image label row is the one that carries the show/hide badge keys.
        if !line.contains("shot.png") || !line.contains("[I]") {
            continue;
        }
        // Click the first cell of the label text (the hit-region is the whole
        // label line, so any cell on the row works).
        for col in 0..area.width {
            if buf[(col, row)].symbol() == "s" {
                badge = Some((col, row));
                break 'rows;
            }
        }
    }
    let (badge_col, badge_row) = badge.expect("image label cell should be visible in the frame");

    assert_eq!(
        app.image_expand_level(image_id),
        ImageExpandLevel::Fit,
        "image should start at Fit before any click"
    );

    // REAL click on the rendered label cell. A terminal delivers a *pair* of
    // events for one physical click: `Down` then `Up`. We must replay both, just
    // like the live event loop, or we silently skip the copy-selection state the
    // `Down` arms (which is exactly what the user's click goes through).
    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: badge_col,
        row: badge_row,
        modifiers: KeyModifiers::empty(),
    });
    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: badge_col,
        row: badge_row,
        modifiers: KeyModifiers::empty(),
    });

    assert_eq!(
        app.image_expand_level(image_id),
        ImageExpandLevel::Large,
        "clicking the rendered image label must cycle Fit -> Large \
         (this is the exact path the user reported as broken)"
    );
    assert_eq!(app.status_notice(), Some("Image size: large".to_string()));
}
/// The inline-image placeholder marker row must never reach the terminal as
/// text. It used to be drawn black-on-black and relied on staying invisible,
/// but terminal-side compositing (kitty translucent background + contrast
/// compositing) and selection highlighting can recolor it, leaking raw
/// "IIMG:<hash>:..." into the transcript whenever the image is not painted
/// over it (cold cache after reload, prewarm in flight, no image protocol).
/// The draw path must blank marker rows instead.
#[test]
fn test_real_draw_never_emits_inline_image_marker_text() {
    use crate::message::{ContentBlock, Role};

    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    assert!(!app.is_remote, "repro must use the local image render path");

    const TOOL_ID: &str = "read-shot-marker";

    app.session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "read the screenshot".to_string(),
            cache_control: None,
        }],
    );
    app.session.add_message(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: TOOL_ID.to_string(),
            name: "read".to_string(),
            input: serde_json::json!({"file_path": "shot.png"}),
            thought_signature: None,
        }],
    );
    app.session.add_message(
        Role::User,
        vec![
            ContentBlock::ToolResult {
                tool_use_id: TOOL_ID.to_string(),
                content: "read image".to_string(),
                is_error: None,
            },
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: REPRO_TINY_PNG_B64.to_string(),
            },
        ],
    );

    app.display_messages = vec![
        DisplayMessage::user("read the screenshot"),
        DisplayMessage::tool(
            "read shot.png",
            crate::message::ToolCall {
                id: TOOL_ID.to_string(),
                name: "read".to_string(),
                input: serde_json::json!({"file_path": "shot.png"}),
                intent: None,
                thought_signature: None,
            },
        ),
    ];
    app.bump_display_messages_version();
    app.invalidate_side_pane_images_signature();
    app.pin_images = true;
    app.inline_images_visible = true;
    app.scroll_offset = 0;
    app.auto_scroll_paused = false;
    app.is_processing = false;
    app.status = ProcessingStatus::Idle;
    app.session.short_name = Some("test".to_string());

    let backend = ratatui::backend::TestBackend::new(80, 40);
    let mut terminal = ratatui::Terminal::new(backend).expect("failed to create test terminal");
    let rendered = render_and_snap(&app, &mut terminal);

    assert!(
        rendered.contains("shot.png"),
        "sanity: the anchored image's label line must render, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("IIMG"),
        "raw inline-image marker text must never be drawn to the terminal, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("MERMAID_IMAGE"),
        "raw mermaid marker text must never be drawn to the terminal, got:\n{rendered}"
    );
}
/// Clicking anywhere on the image body (its placeholder rows) must cycle the
/// expand level, exactly like the label badge. Clicks in the blank area to
/// the RIGHT of a narrow image must not.
#[test]
fn test_click_on_inline_image_body_cycles_level() {
    use crate::tui::ui::inline_image_ui::{
        AllFit, ImageExpandLevel, InlineImageItem, build_section,
    };
    use jcode_tui_messages::PreparedChatFrame;

    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();

    const IMAGE_ID: u64 = 0xBEEF;
    let chat_width: u16 = 80;

    let items = vec![InlineImageItem {
        id: IMAGE_ID,
        width: 320,
        height: 200,
        label: "shot.png".to_string(),
        uses_text_fallback: false,
    }];
    let section = build_section(&items, chat_width, 40, false, true, &AllFit);
    let region = *section
        .image_regions
        .iter()
        .find(|r| r.hash == IMAGE_ID)
        .expect("section should carry the image region");
    assert!(region.width > 0, "fit regions record their rendered width");
    assert!(
        region.width < chat_width,
        "test image must be narrower than the chat so the right side is blank"
    );

    let prepared =
        std::sync::Arc::new(PreparedChatFrame::from_single(std::sync::Arc::new(section)));
    let visible_end = prepared.wrapped_plain_line_count();
    let content_area = Rect::new(0, 0, chat_width, visible_end as u16 + 1);

    crate::tui::ui::clear_copy_viewport_snapshot();
    crate::tui::ui::record_copy_viewport_frame_snapshot_for_test(
        prepared,
        0,
        visible_end,
        content_area,
        &vec![0u16; visible_end],
    );

    assert_eq!(app.image_expand_level(IMAGE_ID), ImageExpandLevel::Fit);

    // Click in the middle of the image body (a placeholder row, inside the
    // rendered width). Down then Up, like a real terminal click.
    let body_row = content_area.y + region.abs_line_idx as u16 + 1;
    let body_col = content_area.x + region.width / 2;
    let click = |app: &mut App, col: u16, row: u16| {
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
    };
    click(&mut app, body_col, body_row);
    assert_eq!(
        app.image_expand_level(IMAGE_ID),
        ImageExpandLevel::Large,
        "clicking the image body should expand Fit -> Large"
    );

    // Large and Full resolve to the same geometry for this landscape image, so
    // the click cycle must omit Full rather than showing a duplicate size.
    click(&mut app, body_col, body_row);
    assert_eq!(
        app.image_expand_level(IMAGE_ID),
        ImageExpandLevel::Fit,
        "second body click should skip duplicate Full geometry and return to Fit"
    );

    // A click in the blank space to the right of the image must stay inert.
    let far_right = content_area.x + chat_width - 2;
    assert!(far_right > content_area.x + region.width);
    click(&mut app, far_right, body_row);
    assert_eq!(
        app.image_expand_level(IMAGE_ID),
        ImageExpandLevel::Fit,
        "clicking blank space beside the image must not cycle it"
    );
}
