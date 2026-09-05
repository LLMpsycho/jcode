use super::*;

fn assert_prompt_fork_route(
    route: Option<crate::config::ConfigModelRoute>,
    api: Option<&str>,
    effort: Option<&str>,
    visible_history: bool,
) {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.is_remote = false;
        app.session.model = Some("chosen-model".into());
        app.session.provider_key = Some("openai-oauth".into());
        app.session.route_api_method = api.map(str::to_owned);
        app.session.reasoning_effort = effort.map(str::to_owned);
        app.session.role_model_selection = route.clone();
        app.session.provider_session_id = Some("parent-provider-conversation".into());
        app.session.autoreview_enabled = Some(true);
        app.session.autojudge_enabled = Some(false);
        if visible_history {
            app.session.add_message(
                crate::message::Role::User,
                vec![crate::message::ContentBlock::Text {
                    text: "Inspect this project".into(),
                    cache_control: None,
                }],
            );
        }
        let parent_id = app.session.id.clone();
        let (child_id, _) = crate::tui::app::commands_review::clone_session_for_prompt(&app)
            .expect("fork local prompt session");
        let mut child = crate::session::Session::load(&child_id).expect("reload fork");
        assert_ne!(child_id, parent_id);
        assert_eq!(child.parent_id.as_deref(), Some(parent_id.as_str()));
        assert_eq!(child.model, app.session.model);
        assert_eq!(child.provider_key, app.session.provider_key);
        assert_eq!(child.route_api_method.as_deref(), api);
        assert_eq!(child.reasoning_effort.as_deref(), effort);
        assert_eq!(child.role_model_selection, route);
        assert_eq!(child.autoreview_enabled, Some(true));
        assert_eq!(child.autojudge_enabled, Some(false));
        assert!(
            child.provider_session_id.is_none(),
            "never reuse the parent's provider conversation"
        );
        child.reasoning_effort = Some("low".into());
        child.role_model_selection = None;
        child.save().expect("save independent child selection");
        assert_eq!(app.session.reasoning_effort.as_deref(), effort);
        assert_eq!(app.session.role_model_selection, route);
        assert_eq!(
            app.session.provider_session_id.as_deref(),
            Some("parent-provider-conversation")
        );
    });
}

#[test]
fn post_merge_review_prompt_fork_preserves_explicit_role_route() {
    assert_prompt_fork_route(
        Some(crate::config::ConfigModelRoute {
            model: "chosen-model".into(),
            api_method: "openai-oauth".into(),
            provider_label: "selected-account".into(),
        }),
        Some("openai-oauth"),
        Some("high"),
        true,
    );
}

#[test]
fn post_merge_review_prompt_fork_preserves_main_route_without_inventing_role() {
    assert_prompt_fork_route(None, Some("openai-oauth"), Some("medium"), true);
}

#[test]
fn post_merge_review_prompt_fork_preserves_legacy_unset_selection() {
    assert_prompt_fork_route(None, None, None, true);
}

#[test]
fn post_merge_review_prompt_fork_persists_before_first_visible_message() {
    assert_prompt_fork_route(None, None, None, false);
}
