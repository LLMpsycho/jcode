use super::*;

#[test]
fn agent_profile_survives_journal_startup_and_remote_resume() -> anyhow::Result<()> {
    let _lock = lock_env();
    let home = tempfile::tempdir()?;
    let _home = EnvVarGuard::set("JCODE_HOME", home.path());
    let mut session = Session::create_with_id(
        "session_profile_roundtrip".into(),
        None,
        Some("Profile worker".into()),
    );
    let profile = SessionAgentProfile {
        name: "debug".into(),
        content: "Diagnose before changing code.".into(),
        allowed_tools: Some(vec!["read".into(), "agentgrep".into()]),
    };
    session.agent_profile = Some(profile.clone());
    session.short_name = Some("debug agent".into());
    session.save()?;
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "Investigate".into(),
            cache_control: None,
        }],
    );
    session.save()?;
    assert!(std::fs::read_to_string(session_journal_path(&session.id)?)?.contains("agent_profile"));
    for loaded in [
        Session::load(&session.id)?,
        Session::load_startup_stub(&session.id)?,
        Session::load_for_remote_startup(&session.id)?,
    ] {
        assert_eq!(loaded.agent_profile.as_ref(), Some(&profile));
        assert_eq!(loaded.display_name(), "debug agent");
        assert_eq!(loaded.id, session.id);
    }
    session.agent_profile = None;
    session.save()?;
    assert!(
        Session::load_startup_stub(&session.id)?
            .agent_profile
            .is_none()
    );
    let legacy = serde_json::to_value(&session)?;
    assert!(legacy.get("agent_profile").is_none());
    assert!(
        serde_json::from_value::<Session>(legacy)?
            .agent_profile
            .is_none()
    );
    Ok(())
}
