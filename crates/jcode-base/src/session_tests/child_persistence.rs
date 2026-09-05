use super::*;

#[test]
fn post_merge_fork_child_is_durable_while_untouched_root_stays_ephemeral() -> anyhow::Result<()> {
    let _lock = lock_env();
    let temporary = tempfile::tempdir()?;
    let _home = EnvVarGuard::set("JCODE_HOME", temporary.path());
    let mut root = Session::create(None, None);
    root.ensure_initial_session_context_message();
    root.save()?;
    assert!(!session_path(&root.id)?.exists());
    let mut child = Session::create(Some(root.id.clone()), None);
    child.ensure_initial_session_context_message();
    child.save()?;
    let stored = Session::load(&child.id)?;
    assert_eq!(stored.parent_id, child.parent_id);
    assert_eq!(stored.messages.len(), child.messages.len());
    assert!(stored.title.is_none());
    assert!(
        !stored.saved,
        "fork persistence must not bookmark the child"
    );
    assert!(stored.provider_session_id.is_none());
    child.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "First child request".into(),
            cache_control: None,
        }],
    );
    child.save()?;
    assert_eq!(
        Session::load(&child.id)?.messages.len(),
        child.messages.len()
    );
    Ok(())
}
