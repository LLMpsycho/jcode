use super::*;

fn parent() -> Session {
    let mut session = Session::create(None, None);
    session.model = Some("parent-model".into());
    session.provider_key = Some("openai-oauth".into());
    session.route_api_method = Some("openai-oauth".into());
    session.reasoning_effort = Some("high".into());
    session
}

#[test]
fn review_role_inherit_preserves_main_route_and_effort() {
    let mut parent = parent();
    parent.role_model_selection = Some(ConfigModelRoute {
        model: "parent-model".into(),
        api_method: "openai-oauth".into(),
        provider_label: "OpenAI".into(),
    });
    let choice = ReviewModelSelection::from_settings(&parent, None, None, None);
    let mut child = parent.clone();
    choice.apply(&mut child);
    assert_eq!(child.model, parent.model);
    assert!(child.role_model_selection.is_none());
    assert_eq!(child.provider_key, parent.provider_key);
    assert_eq!(child.route_api_method, parent.route_api_method);
    assert_eq!(child.reasoning_effort, parent.reasoning_effort);
}

#[test]
fn review_role_explicit_route_wins_over_legacy_model_and_main_effort() {
    let parent = parent();
    let route = ConfigModelRoute {
        model: "selected-model".into(),
        api_method: "claude-oauth".into(),
        provider_label: "Anthropic".into(),
    };
    let choice = ReviewModelSelection::from_settings(
        &parent, Some(&route), Some("stale-legacy-model"), Some("max"),
    );
    let mut child = Session::create(None, None);
    choice.apply(&mut child);
    assert_eq!(child.model.as_deref(), Some("selected-model"));
    assert_eq!(child.role_model_selection.as_ref(), Some(&route));
    assert_eq!(child.provider_key.as_deref(), Some("claude-oauth"));
    assert_eq!(child.route_api_method.as_deref(), Some("claude-oauth"));
    assert_eq!(child.reasoning_effort.as_deref(), Some("max"));
    assert_eq!(parent.model.as_deref(), Some("parent-model"));
    assert_eq!(parent.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(
        crate::provider::MultiProvider::model_switch_request_for_session_route(
            child.model.as_deref().unwrap(),
            child.provider_key.as_deref(),
            child.route_api_method.as_deref(),
        ),
        "claude-oauth:selected-model"
    );
}

#[test]
fn review_role_fixed_model_does_not_inherit_incompatible_parent_effort() {
    let parent = parent();
    let choice = ReviewModelSelection::from_settings(&parent, None, Some("worker-model"), None);
    let mut child = Session::create(None, None);
    choice.apply(&mut child);
    assert_eq!(child.model.as_deref(), Some("worker-model"));
    assert!(child.reasoning_effort.is_none());
    assert!(child.route_api_method.is_none());
    assert!(child.provider_key.is_none());
}

#[test]
fn review_role_persists_custom_endpoint_and_openrouter_provider_pin() {
    for (api, provider, model, expected, restore) in [
        ("openai-compatible:local", "Local", "worker", "worker", "local:worker"),
        ("openrouter", "Anthropic", "anthropic/worker", "anthropic/worker@Anthropic", "openrouter:anthropic/worker@Anthropic"),
        ("jcode-subscription", "Jcode", "managed-worker", "managed-worker", "managed-worker"),
    ] {
        let route = ConfigModelRoute {
            model: model.into(),
            api_method: api.into(),
            provider_label: provider.into(),
        };
        let choice = ReviewModelSelection::from_settings(&parent(), Some(&route), None, Some("low"));
        let mut child = Session::create(None, None);
        choice.apply(&mut child);
        assert_eq!(child.model.as_deref(), Some(expected));
        assert_eq!(child.route_api_method.as_deref(), Some(api));
        assert_eq!(child.reasoning_effort.as_deref(), Some("low"));
        assert_eq!(
            crate::provider::MultiProvider::model_switch_request_for_session_route(
                child.model.as_deref().unwrap(),
                child.provider_key.as_deref(),
                child.route_api_method.as_deref(),
            ),
            restore
        );
    }
}
