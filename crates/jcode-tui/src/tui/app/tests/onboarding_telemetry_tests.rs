#[test]
fn telemetry_pill_opens_settings_page_and_commits_choice() {
    use crate::external_auth::ExternalAuthReviewCandidate;
    use crate::tui::app::onboarding_flow::{ImportReview, TelemetryLevel};

    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.onboarding_flow = None;
        app.begin_onboarding_flow_at_login();
        let review =
            ImportReview::new(vec![ExternalAuthReviewCandidate::fixture("OpenAI/Codex", "Codex auth.json")])
                .unwrap();
        if let Some(flow) = app.onboarding_flow.as_mut() {
            flow.phase = OnboardingPhase::Login {
                import: Some(review),
            };
        }

        // Right twice: Subscription -> Import less -> Telemetry settings.
        assert!(app.handle_onboarding_continue_prompt_key(KeyCode::Right));
        assert!(app.handle_onboarding_continue_prompt_key(KeyCode::Right));
        assert!(app.handle_onboarding_continue_prompt_key(KeyCode::Enter));

        // The page opens defaulted to "Send everything".
        match app.onboarding_phase() {
            Some(OnboardingPhase::Login {
                import: Some(review),
            }) => assert_eq!(review.telemetry, Some(TelemetryLevel::Everything)),
            other => panic!("expected telemetry page open, got {other:?}"),
        }
        // The import countdown is paused while the page is open, so the screen
        // cannot commit the import out from under the user.
        assert!(!app.onboarding_flow.as_ref().unwrap().decision_timed_out());

        // Enter commits "Send everything": usage on, content sharing on.
        assert!(app.handle_onboarding_continue_prompt_key(KeyCode::Enter));
        assert_eq!(
            crate::telemetry::is_enabled(),
            !crate::telemetry::opt_out_forced_by_env()
        );
        assert!(!crate::storage::jcode_dir().unwrap().join("no_telemetry").exists());
        assert!(crate::storage::jcode_dir()
            .unwrap()
            .join("telemetry_share_transcripts_v1")
            .exists());
        assert_eq!(
            crate::telemetry::content_sharing_enabled(),
            std::env::var_os("JCODE_NO_TELEMETRY").is_none()
                && std::env::var_os("DO_NOT_TRACK").is_none()
        );
        // We are back on the summary screen with the import still pending.
        match app.onboarding_phase() {
            Some(OnboardingPhase::Login {
                import: Some(review),
            }) => {
                assert!(review.telemetry.is_none());
                assert!(!review.choosing);
            }
            other => panic!("expected import summary, got {other:?}"),
        }
        assert!(app.onboarding_import_in_progress.is_none());
    });
}

#[test]
fn telemetry_page_send_nothing_disables_telemetry_and_esc_goes_back() {
    use crate::external_auth::ExternalAuthReviewCandidate;
    use crate::tui::app::onboarding_flow::ImportReview;

    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.onboarding_flow = None;
        app.begin_onboarding_flow_at_login();
        let review =
            ImportReview::new(vec![ExternalAuthReviewCandidate::fixture("OpenAI/Codex", "Codex auth.json")])
                .unwrap();
        if let Some(flow) = app.onboarding_flow.as_mut() {
            flow.phase = OnboardingPhase::Login {
                import: Some(review),
            };
        }

        // t is the direct shortcut onto the telemetry page; Esc returns without
        // changing anything and keeps onboarding active.
        assert!(app.handle_onboarding_continue_prompt_key(KeyCode::Char('t')));
        assert!(app.handle_onboarding_continue_prompt_key(KeyCode::Esc));
        assert!(matches!(
            app.onboarding_phase(),
            Some(OnboardingPhase::Login { import: Some(_) })
        ));
        assert_eq!(
            crate::telemetry::is_enabled(),
            !crate::telemetry::opt_out_forced_by_env()
        );
        assert!(!crate::storage::jcode_dir().unwrap().join("no_telemetry").exists());

        // Reopen, walk down to "Send nothing", commit.
        assert!(app.handle_onboarding_continue_prompt_key(KeyCode::Char('t')));
        assert!(app.handle_onboarding_continue_prompt_key(KeyCode::Down));
        assert!(app.handle_onboarding_continue_prompt_key(KeyCode::Down));
        assert!(app.handle_onboarding_continue_prompt_key(KeyCode::Enter));
        assert!(!crate::telemetry::is_enabled());
        assert!(!crate::telemetry::content_sharing_enabled());
    });
}

