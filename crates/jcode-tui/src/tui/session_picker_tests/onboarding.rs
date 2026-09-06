#[test]
fn onboarding_external_filter_picks_latest_visible_transcript() {
    let now = Utc::now();

    let mut older = make_session("codex_older", "older", false, SessionStatus::Closed);
    older.source = SessionSource::Codex;
    older.model = Some("gpt-5-codex".to_string());
    older.last_active_at = Some(now - ChronoDuration::minutes(30));
    older.resume_target = ResumeTarget::CodexSession {
        session_id: "codex_older".to_string(),
        session_path: "/tmp/codex_older.jsonl".to_string(),
    };

    let mut newer = make_session("codex_newer", "newer", false, SessionStatus::Closed);
    newer.source = SessionSource::Codex;
    newer.model = Some("gpt-5-codex".to_string());
    newer.last_active_at = Some(now - ChronoDuration::minutes(2));
    newer.resume_target = ResumeTarget::CodexSession {
        session_id: "codex_newer".to_string(),
        session_path: "/tmp/codex_newer.jsonl".to_string(),
    };

    // A non-Codex session that must be filtered out.
    let jcode = make_session("jcode_one", "jcode", false, SessionStatus::Closed);

    let mut picker = SessionPicker::new(vec![older, jcode, newer]);
    picker.activate_external_cli_filter(SessionFilterMode::Codex);

    assert_eq!(picker.visible_session_count(), 2);

    let latest = picker
        .latest_visible_resume_target()
        .expect("latest visible target");
    assert_eq!(
        latest,
        ResumeTarget::CodexSession {
            session_id: "codex_newer".to_string(),
            session_path: "/tmp/codex_newer.jsonl".to_string(),
        }
    );
}
#[test]
fn onboarding_external_filter_with_no_matches_has_no_target() {
    let jcode = make_session("jcode_only", "jcode", false, SessionStatus::Closed);
    let mut picker = SessionPicker::new(vec![jcode]);
    picker.activate_external_cli_filter(SessionFilterMode::ClaudeCode);

    assert_eq!(picker.visible_session_count(), 0);
    assert!(picker.latest_visible_resume_target().is_none());
}
#[test]
fn onboarding_banner_defaults_to_suggested_review() {
    let mut picker = SessionPicker::new(Vec::new());
    picker.activate_onboarding_banner(vec![Line::from("welcome")]);

    assert!(picker.onboarding_banner_active());
    assert!(picker.onboarding_review_recent_project_highlighted());
    assert!(!picker.onboarding_start_new_highlighted());
}
#[test]
fn onboarding_banner_is_action_only() {
    let mut picker = SessionPicker::new(Vec::new());
    picker.activate_onboarding_banner(vec![Line::from("welcome")]);

    assert_eq!(picker.visible_session_count(), 0);
    assert!(picker.onboarding_review_recent_project_highlighted());
}
#[test]
fn onboarding_banner_offers_review_then_new_session() {
    let mut picker = SessionPicker::new(Vec::new());
    picker.activate_onboarding_banner(vec![Line::from("welcome")]);

    // The suggested review is the default top action.
    let action = picker
        .handle_overlay_key(KeyCode::Enter, KeyModifiers::empty())
        .expect("overlay key");
    assert!(matches!(
        action,
        OverlayAction::Selected(PickerResult::ReviewRecentProject)
    ));

    // Any non-submit key rotates between the two choices.
    picker
        .handle_overlay_key(KeyCode::Char('x'), KeyModifiers::empty())
        .expect("ordinary key");
    assert!(picker.onboarding_start_new_highlighted());
    picker
        .handle_overlay_key(KeyCode::Char('x'), KeyModifiers::empty())
        .expect("ordinary key");
    assert!(picker.onboarding_review_recent_project_highlighted());

    // Keys that normally close the full picker rotate on this action-only page.
    picker
        .handle_overlay_key(KeyCode::Esc, KeyModifiers::empty())
        .expect("escape key");
    assert!(picker.onboarding_start_new_highlighted());
    picker
        .handle_overlay_key(KeyCode::Char('c'), KeyModifiers::CONTROL)
        .expect("control-c");
    assert!(picker.onboarding_review_recent_project_highlighted());

    // Arrow keys use the same rotation behavior.
    picker
        .handle_overlay_key(KeyCode::Down, KeyModifiers::empty())
        .expect("down arrow");
    assert!(picker.onboarding_start_new_highlighted());
    let action = picker
        .handle_overlay_key(KeyCode::Enter, KeyModifiers::empty())
        .expect("overlay key");
    assert!(matches!(
        action,
        OverlayAction::Selected(PickerResult::StartNewSession)
    ));

    // There is no session list below the two actions.
    picker.next();
    assert!(picker.onboarding_start_new_highlighted());
    picker
        .handle_overlay_key(KeyCode::Up, KeyModifiers::empty())
        .expect("up arrow");
    assert!(picker.onboarding_review_recent_project_highlighted());
}
#[test]
fn onboarding_banner_renders_prompt_and_both_action_rows() {
    let mut picker = SessionPicker::new(Vec::new());
    picker.activate_onboarding_banner(vec![
        Line::from("Welcome to jcode"),
        Line::from("Choose how to begin."),
    ]);

    let backend = ratatui::backend::TestBackend::new(120, 40);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| picker.render(frame))
        .expect("render onboarding picker");

    let buffer = terminal.backend().buffer().clone();
    let text: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    let lines = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(
        text.contains("Welcome to jcode"),
        "onboarding prompt should render in the banner: {text:?}"
    );
    assert!(
        text.contains("Start in the current directory"),
        "start-new row should render in the banner: {text:?}"
    );
    assert!(
        text.contains("Find bugs in my most active repo"),
        "suggested-review row should render in the banner: {text:?}"
    );
    assert!(
        !text.contains("Sessions"),
        "resume chrome must be absent: {text:?}"
    );
    assert!(
        !text.contains('╭') && !text.contains('╰') && !text.contains('│'),
        "onboarding choice should not render an outer boundary: {lines:#?}"
    );

    let welcome_y = lines
        .iter()
        .position(|line| line.contains("Welcome to jcode"))
        .expect("welcome row");
    let review_y = lines
        .iter()
        .position(|line| line.contains("Find bugs in my most active repo"))
        .expect("review row");
    let start_y = lines
        .iter()
        .position(|line| line.contains("Start in the current directory"))
        .expect("start-new row");
    let review_x = lines[review_y]
        .find("Find bugs in my most active repo")
        .expect("review column");
    let start_x = lines[start_y]
        .find("Start in the current directory")
        .expect("start-new column");

    assert!(
        welcome_y < review_y,
        "welcome copy should introduce the centered suggested prompt: {lines:#?}"
    );
    assert!(
        review_y.abs_diff(buffer.area.height as usize / 2) <= 1,
        "suggested prompt should be vertically centered: {lines:#?}"
    );
    assert!(
        review_x < 50,
        "suggested prompt should span the visual center: {lines:#?}"
    );
    assert!(
        start_y >= buffer.area.height as usize - 3
            && start_x >= buffer.area.width as usize / 2
            && unicode_width::UnicodeWidthStr::width(lines[start_y].trim_end())
                >= buffer.area.width as usize - 3,
        "blank-session action should stay secondary in the bottom-right: {lines:#?}"
    );
}
