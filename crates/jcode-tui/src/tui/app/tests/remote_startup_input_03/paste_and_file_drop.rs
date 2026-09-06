#[test]
fn test_handle_paste_single_line() {
    let mut app = create_test_app();

    app.handle_paste("hello world".to_string());

    // Small paste (< 5 lines) is inlined directly
    assert_eq!(app.input(), "hello world");
    assert_eq!(app.cursor_pos(), 11);
    assert!(app.pasted_contents.is_empty()); // No placeholder storage needed
}
#[test]
fn test_terminal_file_drop_submits_as_user_input_instead_of_a_skill() {
    let mut app = create_test_app();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("dropped notes.txt");
    std::fs::write(&file, b"notes").unwrap();
    let dropped = file.display().to_string();

    app.handle_paste(dropped.clone());
    assert_eq!(app.input(), dropped);

    app.submit_input();

    assert!(
        app.is_processing,
        "the dropped file path should start a turn"
    );
    assert!(
        app.display_messages()
            .iter()
            .all(|message| message.role != "error"),
        "a dropped absolute path must not produce an unknown-skill error"
    );
    let submitted = app
        .session
        .messages
        .last()
        .expect("submitted file path message");
    assert!(matches!(
        submitted.content.as_slice(),
        [ContentBlock::Text { text, .. }] if text == &file.display().to_string()
    ));
}
#[test]
fn test_terminal_escaped_file_drop_normalizes_the_path() {
    let mut app = create_test_app();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("dropped notes.txt");
    std::fs::write(&file, b"notes").unwrap();
    let escaped = file.display().to_string().replace(' ', "\\ ");

    app.handle_paste(escaped);

    assert_eq!(app.input(), file.display().to_string());
}
#[test]
fn test_terminal_file_drop_with_followup_text_stays_normal_input() {
    let mut app = create_test_app();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("report.md");
    std::fs::write(&file, b"report").unwrap();
    let prompt = format!("{} please review this file", file.display());
    app.set_input_for_test(prompt.clone());

    app.submit_input();

    assert!(app.is_processing, "the file prompt should start a turn");
    assert!(
        app.display_messages()
            .iter()
            .all(|message| message.role != "error"),
        "a path followed by instructions must not be parsed as a skill"
    );
    let submitted = app
        .session
        .messages
        .last()
        .expect("submitted file prompt message");
    assert!(matches!(
        submitted.content.as_slice(),
        [ContentBlock::Text { text, .. }] if text == &prompt
    ));
}
#[test]
fn test_mixed_file_and_image_drop_keeps_file_and_attaches_image() {
    let mut app = create_test_app();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes with spaces.txt");
    let image = dir.path().join("screenshot.png");
    std::fs::write(&file, b"notes").unwrap();
    std::fs::write(&image, b"png bytes").unwrap();
    let dropped = format!(
        "{} {}",
        file.display().to_string().replace(' ', "\\ "),
        image.display()
    );

    app.handle_paste(dropped);

    assert_eq!(app.input(), format!("\"{}\" [image 1]", file.display()));
    assert_eq!(app.pending_images.len(), 1);
    assert_eq!(app.pending_images[0].0, "image/png");
}
#[test]
fn test_terminal_image_drop_attaches_image_instead_of_routing_as_a_skill() {
    let mut app = create_test_app();
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("dropped screenshot.png");
    std::fs::write(&image, b"png bytes").unwrap();

    app.handle_paste(image.display().to_string());

    assert_eq!(app.input(), "[image 1]");
    assert_eq!(app.pending_images.len(), 1);
    assert_eq!(app.pending_images[0].0, "image/png");
    assert!(
        app.display_messages()
            .iter()
            .all(|message| message.role != "error")
    );
}
#[test]
fn test_typed_absolute_image_path_promotes_before_slash_routing() {
    let mut app = create_test_app();
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("dropped photo.png");
    std::fs::write(&image, b"png bytes").unwrap();
    app.set_input_for_test(image.display().to_string());

    assert!(crate::tui::app::input::promote_dropped_images(&mut app));
    assert_eq!(app.input(), "[image 1]");
    assert_eq!(app.pending_images.len(), 1);
    assert_eq!(app.pending_images[0].0, "image/png");
}
#[test]
fn test_incremental_terminal_drop_promotes_immediately_when_path_completes() {
    let mut app = create_test_app();
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("instant.png");
    std::fs::write(&image, b"png bytes").unwrap();

    for ch in image.display().to_string().chars() {
        crate::tui::app::input::handle_text_input(&mut app, &ch.to_string());
    }

    assert_eq!(app.input(), "[image 1]");
    assert_eq!(app.pending_images.len(), 1);
}
#[test]
fn test_handle_paste_multi_line() {
    let mut app = create_test_app();

    app.handle_paste("line 1\nline 2\nline 3".to_string());

    // Small paste (< 5 lines) is inlined directly
    assert_eq!(app.input(), "line 1\nline 2\nline 3");
    assert!(app.pasted_contents.is_empty());
}
#[test]
fn test_handle_paste_large() {
    let mut app = create_test_app();

    app.handle_paste("a\nb\nc\nd\ne".to_string());

    // Large paste (5+ lines) uses placeholder
    assert_eq!(app.input(), "[pasted 5 lines]");
    assert_eq!(app.pasted_contents.len(), 1);
}
#[test]
fn test_paste_again_expands_placeholder_in_place() {
    let mut app = create_test_app();
    let big = "α\nβ\nγ\nδ\nε".to_string();

    app.handle_paste(big.clone());
    app.handle_key(KeyCode::Char('!'), KeyModifiers::empty())
        .unwrap();
    app.handle_paste(big.clone());

    assert_eq!(app.input(), format!("{big}!"));
    assert_eq!(app.cursor_pos, big.len());
    assert!(app.pasted_contents.is_empty());
}
#[test]
fn test_paste_again_with_different_text_still_collapses() {
    let mut app = create_test_app();

    app.handle_paste("a\nb\nc\nd\ne".to_string());
    app.handle_paste("f\ng\nh\ni\nj".to_string());

    assert_eq!(app.input(), "[pasted 5 lines][pasted 5 lines]");
    assert_eq!(app.pasted_contents.len(), 2);
}
#[test]
fn test_paste_again_expands_matching_placeholder_not_newer_same_sized_paste() {
    let mut app = create_test_app();
    let first = "a\nb\nc\nd\ne".to_string();
    let second = "f\ng\nh\ni\nj".to_string();

    app.handle_paste(first.clone());
    app.handle_key(KeyCode::Char(' '), KeyModifiers::empty())
        .unwrap();
    app.handle_paste(second.clone());
    app.handle_paste(first.clone());

    assert_eq!(app.input(), format!("{first} [pasted 5 lines]"));
    assert_eq!(app.cursor_pos, first.len());
    let visible_input = app.input().to_string();
    assert_eq!(
        crate::tui::app::input::expand_paste_placeholders(&mut app, &visible_input),
        format!("{first} {second}")
    );
    assert_eq!(app.pasted_contents, vec![second]);
}
#[test]
fn test_paste_again_expands_only_most_recent_identical_placeholder() {
    let mut app = create_test_app();
    let big = "a\nb\nc\nd\ne".to_string();

    app.set_input_for_test("[pasted 5 lines] [pasted 5 lines]");
    app.pasted_contents = vec![big.clone(), big.clone()];
    app.handle_paste(big.clone());

    assert_eq!(app.input(), format!("[pasted 5 lines] {big}"));
    assert_eq!(app.pasted_contents, vec![big]);
}
#[test]
fn test_paste_again_does_not_expand_an_edited_placeholder() {
    let mut app = create_test_app();
    let big = "a\nb\nc\nd\ne".to_string();

    app.handle_paste(big.clone());
    app.handle_key(KeyCode::Backspace, KeyModifiers::empty())
        .unwrap();
    app.handle_paste(big);

    assert_eq!(
        app.input(),
        "[pasted 5 lines[pasted 5 lines]",
        "an edited placeholder must not be mistaken for the original"
    );
    assert_eq!(app.pasted_contents.len(), 2);
}
#[test]
fn test_paste_expansion_on_submit() {
    let mut app = create_test_app();

    // Type prefix, paste large content, type suffix
    app.handle_key(KeyCode::Char('A'), KeyModifiers::empty())
        .unwrap();
    app.handle_key(KeyCode::Char(':'), KeyModifiers::empty())
        .unwrap();
    app.handle_key(KeyCode::Char(' '), KeyModifiers::empty())
        .unwrap();
    // Paste 5 lines to trigger placeholder
    app.handle_paste("1\n2\n3\n4\n5".to_string());
    app.handle_key(KeyCode::Char(' '), KeyModifiers::empty())
        .unwrap();
    app.handle_key(KeyCode::Char('B'), KeyModifiers::empty())
        .unwrap();

    // Input shows placeholder
    assert_eq!(app.input(), "A: [pasted 5 lines] B");

    // Submit expands placeholder
    app.submit_input();

    // Sent transcript renders the actual pasted content, while the composer above stayed compact.
    assert_eq!(app.display_messages().len(), 1);
    assert_eq!(app.display_messages()[0].content, "A: 1\n2\n3\n4\n5 B");

    // Model receives expanded content (actual pasted text). Local sessions keep the
    // provider message cache lazy, so inspect the materialized provider view.
    let provider_messages = app.materialized_provider_messages();
    let user_message = provider_messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)
        .expect("expected submitted user message");
    match &user_message.content[0] {
        crate::message::ContentBlock::Text { text, .. } => {
            assert_eq!(text, "A: 1\n2\n3\n4\n5 B");
        }
        _ => panic!("Expected Text content block"),
    }

    // Pasted contents should be cleared
    assert!(app.pasted_contents.is_empty());
}
#[test]
fn test_multiple_pastes() {
    let mut app = create_test_app();

    // Small pastes are inlined
    app.handle_paste("first".to_string());
    app.handle_key(KeyCode::Char(' '), KeyModifiers::empty())
        .unwrap();
    app.handle_paste("second\nline".to_string());

    // Both small pastes inlined directly
    assert_eq!(app.input(), "first second\nline");
    assert!(app.pasted_contents.is_empty());

    app.submit_input();
    // Display and model both get the same content (no expansion needed)
    assert_eq!(app.display_messages()[0].content, "first second\nline");
    let provider_messages = app.materialized_provider_messages();
    let user_message = provider_messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)
        .expect("expected submitted user message");
    match &user_message.content[0] {
        crate::message::ContentBlock::Text { text, .. } => {
            assert_eq!(text, "first second\nline");
        }
        _ => panic!("Expected Text content block"),
    }
}
