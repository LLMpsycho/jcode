use super::*;
use crate::config::ConfigModelRoute;

fn selected_route(api_method: &str, provider_label: &str) -> ConfigModelRoute {
    ConfigModelRoute {
        model: "worker-model".to_string(),
        api_method: api_method.to_string(),
        provider_label: provider_label.to_string(),
    }
}

#[test]
fn role_model_marker_is_optional_for_legacy_session_json() -> Result<()> {
    let _lock = lock_env();
    let legacy = Session::create_with_id("session_role_model_legacy".to_string(), None, None);
    let json = serde_json::to_value(&legacy)?;
    assert!(json.get("role_model_selection").is_none());
    let loaded: Session = serde_json::from_value(json.clone())?;
    assert!(loaded.role_model_selection.is_none());
    let stub: SessionStartupStub = serde_json::from_value(json.clone())?;
    assert!(
        Session::session_from_startup_stub(stub)
            .role_model_selection
            .is_none()
    );
    let remote: RemoteStartupSessionSnapshot = serde_json::from_value(json)?;
    assert!(
        Session::session_from_remote_startup_snapshot(remote)
            .role_model_selection
            .is_none()
    );

    let meta_json = serde_json::to_value(legacy.journal_meta())?;
    assert!(meta_json.get("role_model_selection").is_none());
    let meta: SessionJournalMeta = serde_json::from_value(meta_json)?;
    assert!(meta.role_model_selection.is_none());
    Ok(())
}

#[test]
fn role_model_route_and_effort_survive_snapshot_journal_and_startup_loads() -> Result<()> {
    let _lock = lock_env();
    let home = tempfile::tempdir()?;
    let _home = EnvVarGuard::set("JCODE_HOME", home.path());
    let id = "session_role_model_roundtrip";
    let mut session = Session::create_with_id(id.to_string(), None, None);
    let selected = selected_route("openai-compatible:private-endpoint", "Private endpoint");
    session.role_model_selection = Some(selected.clone());
    session.model = Some(selected.model.clone());
    session.reasoning_effort = Some("high".to_string());
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "initial task".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "task result".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;

    let journal = std::fs::read_to_string(session_journal_path(id)?)?;
    assert!(journal.contains("role_model_selection"));
    assert!(journal.contains("openai-compatible:private-endpoint"));
    for loaded in [
        Session::load(id)?,
        Session::load_startup_stub(id)?,
        Session::load_for_remote_startup(id)?,
    ] {
        assert_eq!(loaded.role_model_selection.as_ref(), Some(&selected));
        assert_eq!(loaded.reasoning_effort.as_deref(), Some("high"));
    }
    assert_eq!(Session::load(id)?.messages.len(), 2);
    assert_eq!(Session::load_for_remote_startup(id)?.messages.len(), 2);
    Ok(())
}

#[test]
fn role_model_route_change_and_clear_checkpoint_before_stub_resume() -> Result<()> {
    let _lock = lock_env();
    let home = tempfile::tempdir()?;
    let _home = EnvVarGuard::set("JCODE_HOME", home.path());
    let id = "session_role_model_checkpoint";
    let mut session = Session::create_with_id(id.to_string(), None, None);
    session.role_model_selection = Some(selected_route("openai-oauth", "OpenAI"));
    session.reasoning_effort = Some("high".to_string());
    session.save()?;

    let pinned = selected_route("openrouter", "DeepInfra");
    session.role_model_selection = Some(pinned.clone());
    session.save()?;
    assert_eq!(
        Session::load_startup_stub(id)?.role_model_selection,
        Some(pinned)
    );

    session.role_model_selection = None;
    session.save()?;
    for loaded in [
        Session::load(id)?,
        Session::load_startup_stub(id)?,
        Session::load_for_remote_startup(id)?,
    ] {
        assert!(loaded.role_model_selection.is_none());
        assert_eq!(loaded.reasoning_effort.as_deref(), Some("high"));
    }
    Ok(())
}
