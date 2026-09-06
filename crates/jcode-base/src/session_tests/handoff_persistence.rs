use super::*;

#[test]
fn explicit_handoff_persists_empty_identity_without_bookmarking() -> anyhow::Result<()> {
    let _lock = lock_env();
    let home = tempfile::tempdir()?;
    let _home = EnvVarGuard::set("JCODE_HOME", home.path());
    let _test_session = EnvVarGuard::set("JCODE_TEST_SESSION", "1");
    let mut session = Session::create(None, None);
    session.ensure_initial_session_context_message();
    assert!(session.is_debug);
    session.save()?;
    assert!(!session_path(&session.id)?.exists());

    session.set_canary("handoff-build");
    session.save_for_handoff()?;
    let loaded = Session::load(&session.id)?;
    assert!(loaded.is_debug);
    assert!(loaded.is_canary);
    assert_eq!(loaded.testing_build.as_deref(), Some("handoff-build"));
    assert_eq!(loaded.messages.len(), session.messages.len());
    assert!(!loaded.saved);
    assert!(loaded.title.is_none());
    assert!(loaded.custom_title.is_none());
    assert!(loaded.parent_id.is_none());

    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "First request after handoff".into(),
            cache_control: None,
        }],
    );
    session.save()?;
    assert!(session_journal_path(&session.id)?.exists());
    let loaded = Session::load(&session.id)?;
    assert_eq!(loaded.messages.len(), session.messages.len());
    assert!(loaded.is_debug && loaded.is_canary);
    Ok(())
}

#[test]
fn failed_handoff_does_not_claim_a_snapshot_and_can_be_retried() -> anyhow::Result<()> {
    let _lock = lock_env();
    let home = tempfile::tempdir()?;
    let _home = EnvVarGuard::set("JCODE_HOME", home.path());
    let mut session = Session::create(None, None);
    session.ensure_initial_session_context_message();
    let sessions_path = home.path().join("sessions");
    std::fs::write(&sessions_path, "a file cannot contain session snapshots")?;

    assert!(session.save_for_handoff().is_err());
    assert!(!session.persist_state.snapshot_exists);
    std::fs::remove_file(sessions_path)?;
    session.save()?;
    assert!(!session_path(&session.id)?.exists());
    session.save_for_handoff()?;
    assert_eq!(Session::load(&session.id)?.id, session.id);
    assert!(!session.saved);
    assert!(session.title.is_none());
    Ok(())
}

#[test]
fn handoff_preserves_destructive_empty_checkpoint_protection() -> anyhow::Result<()> {
    let _lock = lock_env();
    let home = tempfile::tempdir()?;
    let _home = EnvVarGuard::set("JCODE_HOME", home.path());
    let mut session = Session::create(None, None);
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "A transcript that must survive".into(),
            cache_control: None,
        }],
    );
    session.save()?;
    session.messages.clear();
    let error = session.save_for_handoff().unwrap_err();
    assert!(error.to_string().contains("refusing to replace non-empty"));
    assert_eq!(Session::load(&session.id)?.messages.len(), 1);
    Ok(())
}
