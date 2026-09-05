use super::*;

#[test]
fn test_review_role_split_snapshots_selected_model_route_and_effort() {
    with_temp_jcode_home(|| {
        use crate::config::{AgentModelRole, Config, ConfigModelRoute};
        let mut app = create_test_app();
        app.is_remote = true;
        app.session.model = Some("main-model".to_string());
        app.session.reasoning_effort = Some("high".to_string());
        let parent_id = app.session.id.clone();
        for (label, role, model, api, effort) in [
            (
                "Review",
                AgentModelRole::Review,
                "review-model",
                "claude-oauth",
                "max",
            ),
            (
                "Autoreview",
                AgentModelRole::Review,
                "auto-review-model",
                "claude-oauth",
                "low",
            ),
            (
                "Judge",
                AgentModelRole::Judge,
                "judge-model",
                "openai-oauth",
                "high",
            ),
            (
                "Autojudge",
                AgentModelRole::Judge,
                "auto-judge-model",
                "openai-oauth",
                "low",
            ),
        ] {
            let route = ConfigModelRoute {
                model: model.to_string(),
                api_method: api.to_string(),
                provider_label: "selected-provider".to_string(),
            };
            Config::set_agent_model_selection(role, Some(&route), Some(model), Some(effort))
                .expect("save role selection");
            crate::tui::app::commands::queue_review_spawn_remote(
                &mut app,
                label,
                parent_id.clone(),
                "review this work".to_string(),
            )
            .expect("queue selected role");
            let selection = app.pending_split_role_selection.take();
            // Saving another role default while a split is in flight must not
            // change the model chosen for that already-queued child.
            Config::set_agent_model_selection(role, None, None, None)
                .expect("clear future role default");
            let mut child = crate::session::Session::create(Some(parent_id.clone()), None);
            child.save().expect("save split child");
            crate::tui::app::commands::prepare_review_spawned_session(
                &child.id,
                app.pending_split_startup_message
                    .take()
                    .expect("startup prompt"),
                selection,
                Some(label.to_ascii_lowercase()),
                Some(parent_id.clone()),
            )
            .expect("prepare selected role");
            let restored = crate::session::Session::load(&child.id).expect("reload child");
            assert_eq!(restored.model.as_deref(), Some(model));
            assert_eq!(restored.role_model_selection.as_ref(), Some(&route));
            assert_eq!(restored.route_api_method.as_deref(), Some(api));
            assert_eq!(restored.provider_key.as_deref(), Some(api));
            assert_eq!(restored.reasoning_effort.as_deref(), Some(effort));
            assert_eq!(restored.autoreview_enabled, Some(false));
            assert_eq!(restored.autojudge_enabled, Some(false));
            assert_eq!(app.session.model.as_deref(), Some("main-model"));
            assert_eq!(app.session.reasoning_effort.as_deref(), Some("high"));
        }
    });
}

#[test]
fn test_review_role_inherit_does_not_prefer_an_available_oauth_account() {
    with_temp_jcode_home(|| {
        let auth_path = crate::storage::jcode_dir()
            .expect("jcode dir")
            .join("openai-auth.json");
        std::fs::write(
            auth_path,
            serde_json::json!({
                "openai_accounts": [{
                    "label": "openai-1",
                    "access_token": "at_test",
                    "refresh_token": "rt_test",
                    "account_id": "acct_test"
                }],
                "active_openai_account": "openai-1"
            })
            .to_string(),
        )
        .expect("write unrelated auth account");
        let mut app = create_test_app();
        app.is_remote = true;
        app.session.model = Some("local-placeholder-model".to_string());
        app.session.provider_key = Some("claude-oauth".to_string());
        app.session.route_api_method = Some("claude-oauth".to_string());
        app.remote_reasoning_effort = Some("max".to_string());
        let parent_id = app.session.id.clone();
        crate::tui::app::commands::queue_review_spawn_remote(
            &mut app,
            "Review",
            parent_id.clone(),
            "review".to_string(),
        )
        .expect("queue inherited review");
        let child = crate::session::Session::create(Some(parent_id), None);
        let mut child = child;
        child.model = Some("chosen-main-model".to_string());
        child.provider_key = Some("claude-oauth".to_string());
        child.route_api_method = Some("claude-oauth".to_string());
        child.save().expect("save child");
        crate::tui::app::commands::prepare_review_spawned_session(
            &child.id,
            "review".to_string(),
            app.pending_split_role_selection.take(),
            Some("review".to_string()),
            None,
        )
        .expect("prepare inherited review");
        let restored = crate::session::Session::load(&child.id).expect("reload child");
        assert_eq!(restored.model.as_deref(), Some("chosen-main-model"));
        assert!(restored.role_model_selection.is_none());
        assert_eq!(restored.provider_key.as_deref(), Some("claude-oauth"));
        assert_eq!(restored.reasoning_effort.as_deref(), Some("max"));
    });
}

#[test]
fn test_prepare_review_role_reports_missing_child_instead_of_launching_inherited_model() {
    with_temp_jcode_home(|| {
        let result = crate::tui::app::commands::prepare_review_spawned_session(
            "missing_review_child",
            "review".to_string(),
            None,
            Some("review".to_string()),
            None,
        );
        assert!(result.is_err());
    });
}
