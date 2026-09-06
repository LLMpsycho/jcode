#[test]
fn test_session_exists_roundtrip() -> Result<()> {
    let tmp_dir = std::env::temp_dir().join(format!(
        "jcode-session-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| anyhow!(e))?
            .as_nanos()
    ));
    std::fs::create_dir_all(tmp_dir.join("sessions"))?;

    assert!(!session_path_in_dir(&tmp_dir, "missing-session").exists());

    let session_path = session_path_in_dir(&tmp_dir, "exists-session");
    std::fs::write(&session_path, "{}")?;
    assert!(session_path.exists());

    let random_id = format!(
        "missing-session-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| anyhow!(e))?
            .as_nanos()
    );
    assert!(!session_exists(&random_id));
    Ok(())
}
#[test]
fn derive_session_provider_key_prefers_runtime_identity_over_transport() {
    let _lock = lock_env();
    let _runtime = EnvVarGuard::set("JCODE_RUNTIME_PROVIDER", "azure-openai");
    let _namespace = EnvVarGuard::set("JCODE_OPENROUTER_CACHE_NAMESPACE", "azure-cache");
    let _active = EnvVarGuard::set("JCODE_ACTIVE_PROVIDER", "openrouter");

    assert_eq!(
        derive_session_provider_key("openrouter").as_deref(),
        Some("azure-openai")
    );
}
#[test]
fn derive_session_provider_key_falls_back_to_openrouter_namespace() {
    let _lock = lock_env();
    let _runtime = EnvVarGuard::remove("JCODE_RUNTIME_PROVIDER");
    let _namespace = EnvVarGuard::set("JCODE_OPENROUTER_CACHE_NAMESPACE", "azure-openai");
    let _active = EnvVarGuard::set("JCODE_ACTIVE_PROVIDER", "openrouter");

    assert_eq!(
        derive_session_provider_key("openrouter").as_deref(),
        Some("azure-openai")
    );
}
#[test]
fn derive_session_provider_key_keeps_openai_compatible_profile_namespace() {
    let _lock = lock_env();
    let _runtime = EnvVarGuard::set("JCODE_RUNTIME_PROVIDER", "openai-compatible");
    let _namespace = EnvVarGuard::set("JCODE_OPENROUTER_CACHE_NAMESPACE", "zai");
    let _active = EnvVarGuard::set("JCODE_ACTIVE_PROVIDER", "openrouter");

    assert_eq!(
        derive_session_provider_key("openrouter").as_deref(),
        Some("zai")
    );
}
#[test]
fn rename_title_preserves_generated_title_for_clear() {
    let mut session = Session::create_with_id(
        "session_rename_clear_123".to_string(),
        None,
        Some("Generated first prompt title".to_string()),
    );

    assert_eq!(
        session.display_title(),
        Some("Generated first prompt title")
    );
    session.rename_title(Some("Custom planning name".to_string()));
    assert_eq!(
        session.title.as_deref(),
        Some("Generated first prompt title")
    );
    assert_eq!(
        session.custom_title.as_deref(),
        Some("Custom planning name")
    );
    assert_eq!(session.display_title(), Some("Custom planning name"));

    session.rename_title(None);
    assert_eq!(
        session.title.as_deref(),
        Some("Generated first prompt title")
    );
    assert!(session.custom_title.is_none());
    assert_eq!(
        session.display_title(),
        Some("Generated first prompt title")
    );

    session.custom_title = Some("   ".to_string());
    assert_eq!(
        session.display_title(),
        Some("Generated first prompt title")
    );
}
#[test]
fn test_debug_memory_profile_reports_messages_and_provider_cache() {
    let mut session = Session::create_with_id(
        "session_memory_profile_test".to_string(),
        None,
        Some("Memory profile".to_string()),
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "hello world".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::Assistant,
        vec![
            ContentBlock::ToolUse {
                id: "tool_1".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"command": "echo hi"}),
                thought_signature: None,
            },
            ContentBlock::ToolResult {
                tool_use_id: "tool_1".to_string(),
                content: "hi".to_string(),
                is_error: None,
            },
        ],
    );

    session.compaction = Some(StoredCompactionState {
        summary_text: "summary".to_string(),
        openai_encrypted_content: None,
        covers_up_to_turn: 7,
        original_turn_count: 9,
        compacted_count: 7,
    });

    let _ = session.provider_messages();
    let profile = session.debug_memory_profile();

    assert_eq!(profile["messages"]["count"], 2);
    assert_eq!(profile["messages"]["memory"]["text_blocks"], 1);
    assert_eq!(profile["messages"]["memory"]["tool_use_blocks"], 1);
    assert_eq!(profile["messages"]["memory"]["tool_result_blocks"], 1);
    assert!(profile["messages"]["json_bytes"].as_u64().unwrap_or(0) > 0);
    assert_eq!(profile["provider_messages_cache"]["count"], 2);
    assert_eq!(profile["compaction"]["present"], true);
    assert_eq!(profile["compaction"]["covers_up_to_turn"], 7);
    assert_eq!(profile["compaction"]["original_turn_count"], 9);
    assert_eq!(profile["compaction"]["compacted_count"], 7);
    assert!(
        profile["provider_messages_cache"]["json_bytes"]
            .as_u64()
            .unwrap_or(0)
            > 0
    );
}
#[test]
fn released_provider_message_cache_rebuilds_from_canonical_history() {
    let mut session = Session::create_with_id(
        "session_provider_cache_release_test".to_string(),
        None,
        Some("Provider cache release".to_string()),
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "large derived payload".repeat(1024),
            cache_control: None,
        }],
    );

    let before = session.provider_messages().to_vec();
    assert_eq!(before.len(), 1);
    assert_eq!(
        session.debug_memory_profile()["provider_messages_cache"]["count"],
        1
    );

    session.release_provider_messages_cache();
    let released = session.debug_memory_profile();
    assert_eq!(released["provider_messages_cache"]["count"], 0);
    assert_eq!(released["provider_messages_cache"]["json_bytes"], 0);

    let rebuilt = session.provider_messages();
    assert_eq!(rebuilt.len(), 1);
    assert_eq!(
        serde_json::to_value(rebuilt).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}
#[test]
fn token_usage_totals_counts_cache_reported_inputs_only_when_cache_fields_exist() {
    let mut session = Session::create_with_id(
        "session_token_usage_totals_test".to_string(),
        None,
        Some("Token totals".to_string()),
    );
    session.add_message_ext(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "first".to_string(),
            cache_control: None,
        }],
        None,
        Some(StoredTokenUsage {
            input_tokens: 100,
            output_tokens: 10,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        }),
    );
    session.add_message_ext(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "second".to_string(),
            cache_control: None,
        }],
        None,
        Some(StoredTokenUsage {
            input_tokens: 200,
            output_tokens: 20,
            cache_read_input_tokens: Some(150),
            cache_creation_input_tokens: Some(25),
        }),
    );

    let totals = session.token_usage_totals();
    assert_eq!(totals.messages_with_token_usage, 2);
    assert_eq!(totals.input_tokens, 300);
    assert_eq!(totals.output_tokens, 30);
    assert_eq!(totals.cache_reported_input_tokens, 200);
    assert_eq!(totals.cache_read_input_tokens, 150);
    assert_eq!(totals.cache_creation_input_tokens, 25);
}
#[test]
fn initial_session_context_is_persisted_once_and_not_overwritten() {
    let mut session = Session::create_with_id(
        "session_context_test".to_string(),
        None,
        Some("Session context".to_string()),
    );

    assert!(session.ensure_initial_session_context_message());
    assert_eq!(session.messages.len(), 1);
    let first = session.messages[0].content_preview();
    assert!(first.contains("# Session Context"));
    assert!(first.contains("OS:"));
    assert_eq!(
        session.messages[0].display_role,
        Some(StoredDisplayRole::System)
    );

    assert!(!session.ensure_initial_session_context_message());
    assert_eq!(session.messages.len(), 1);

    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "hello".to_string(),
            cache_control: None,
        }],
    );
    assert!(!session.ensure_initial_session_context_message());
    assert_eq!(session.messages.len(), 2);
}
#[test]
#[allow(clippy::redundant_closure_call)]
fn initial_session_context_preserves_explicitly_bound_cwd_when_inserted() -> Result<()> {
    let _env_lock = lock_env();
    let original_cwd = std::env::current_dir().map_err(|e| anyhow!(e))?;
    let first_dir = tempfile::Builder::new()
        .prefix("jcode-session-context-first-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let second_dir = tempfile::Builder::new()
        .prefix("jcode-session-context-second-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;

    std::env::set_current_dir(first_dir.path()).map_err(|e| anyhow!(e))?;
    let mut session = Session::create_with_id(
        "session_context_cwd_refresh_test".to_string(),
        None,
        Some("Session context cwd refresh".to_string()),
    );
    assert_eq!(
        session.working_dir.as_deref(),
        Some(first_dir.path().to_str().unwrap())
    );

    std::env::set_current_dir(second_dir.path()).map_err(|e| anyhow!(e))?;
    let result: std::result::Result<(), anyhow::Error> = (|| {
        assert!(session.ensure_initial_session_context_message());
        let first = session.messages[0].content_preview();
        assert!(
            first.contains(&format!(
                "Working directory: {}",
                first_dir.path().display()
            )),
            "session context should preserve the bound cwd, got: {first}"
        );
        assert_eq!(
            session.working_dir.as_deref(),
            Some(first_dir.path().to_str().unwrap())
        );
        Ok(())
    })();
    std::env::set_current_dir(original_cwd).map_err(|e| anyhow!(e))?;
    result?;

    Ok(())
}
#[test]
#[allow(clippy::redundant_closure_call)]
fn initial_session_context_can_refresh_before_real_conversation() -> Result<()> {
    let _env_lock = lock_env();
    let original_cwd = std::env::current_dir().map_err(|e| anyhow!(e))?;
    let first_dir = tempfile::Builder::new()
        .prefix("jcode-session-context-stale-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let second_dir = tempfile::Builder::new()
        .prefix("jcode-session-context-real-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;

    std::env::set_current_dir(first_dir.path()).map_err(|e| anyhow!(e))?;
    let result: std::result::Result<(), anyhow::Error> = (|| {
        let mut session = Session::create_with_id(
            "session_context_remote_cwd_refresh_test".to_string(),
            None,
            Some("Remote cwd refresh".to_string()),
        );
        assert!(session.ensure_initial_session_context_message());
        assert!(session.messages[0].content_preview().contains(&format!(
            "Working directory: {}",
            first_dir.path().display()
        )));

        session.working_dir = Some(second_dir.path().display().to_string());
        assert!(session.refresh_initial_session_context_message());
        let refreshed = session.messages[0].content_preview();
        assert!(
            refreshed.contains(&format!(
                "Working directory: {}",
                second_dir.path().display()
            )),
            "session context should refresh to subscribed cwd, got: {refreshed}"
        );
        assert!(!refreshed.contains(&format!(
            "Working directory: {}",
            first_dir.path().display()
        )));
        Ok(())
    })();
    std::env::set_current_dir(original_cwd).map_err(|e| anyhow!(e))?;
    result?;

    Ok(())
}
#[test]
#[allow(clippy::redundant_closure_call)]
fn initial_session_context_does_not_refresh_after_real_conversation() -> Result<()> {
    let _env_lock = lock_env();
    let original_cwd = std::env::current_dir().map_err(|e| anyhow!(e))?;
    let first_dir = tempfile::Builder::new()
        .prefix("jcode-session-context-original-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let second_dir = tempfile::Builder::new()
        .prefix("jcode-session-context-late-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;

    std::env::set_current_dir(first_dir.path()).map_err(|e| anyhow!(e))?;
    let result: std::result::Result<(), anyhow::Error> = (|| {
        let mut session = Session::create_with_id(
            "session_context_late_cwd_refresh_test".to_string(),
            None,
            Some("Late cwd refresh".to_string()),
        );
        assert!(session.ensure_initial_session_context_message());
        session.add_message(
            Role::User,
            vec![ContentBlock::Text {
                text: "hello".to_string(),
                cache_control: None,
            }],
        );

        session.working_dir = Some(second_dir.path().display().to_string());
        assert!(!session.refresh_initial_session_context_message());
        let original = session.messages[0].content_preview();
        assert!(original.contains(&format!(
            "Working directory: {}",
            first_dir.path().display()
        )));
        assert!(!original.contains(&format!(
            "Working directory: {}",
            second_dir.path().display()
        )));
        Ok(())
    })();
    std::env::set_current_dir(original_cwd).map_err(|e| anyhow!(e))?;
    result?;

    Ok(())
}
#[test]
fn existing_non_empty_session_does_not_get_retroactive_session_context() {
    let mut session = Session::create_with_id(
        "session_context_existing_test".to_string(),
        None,
        Some("Existing".to_string()),
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "already started".to_string(),
            cache_control: None,
        }],
    );

    assert!(!session.ensure_initial_session_context_message());
    assert_eq!(session.messages.len(), 1);
    assert!(
        !session.messages[0]
            .content_preview()
            .contains("# Session Context")
    );
}
#[test]
fn load_startup_stub_preserves_metadata_but_skips_heavy_vectors() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-startup-stub-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let session_id = "session_startup_stub_roundtrip";
    let mut session = Session::create_with_id(
        session_id.to_string(),
        Some("parent_123".to_string()),
        Some("startup stub".to_string()),
    );
    session.model = Some("gpt-5.4".to_string());
    session.reasoning_effort = Some("high".to_string());
    session.provider_key = Some("openai".to_string());
    session.route_api_method = Some("openai-api".to_string());
    session.set_canary("self-dev");
    session.append_stored_message(StoredMessage {
        id: "msg_1".to_string(),
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: "hello world".to_string(),
            cache_control: None,
        }],
        display_role: None,
        timestamp: Some(Utc::now()),
        tool_duration_ms: None,
        token_usage: None,
    });
    session.record_env_snapshot(EnvSnapshot {
        captured_at: Utc::now(),
        reason: "resume".to_string(),
        session_id: session_id.to_string(),
        working_dir: Some(temp_home.path().to_string_lossy().to_string()),
        provider: "openai".to_string(),
        model: "gpt-5.4".to_string(),
        jcode_version: "test".to_string(),
        jcode_git_hash: Some("abc123".to_string()),
        jcode_git_dirty: Some(false),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        pid: 123,
        is_selfdev: true,
        is_debug: false,
        is_canary: true,
        testing_build: Some("self-dev".to_string()),
        working_git: None,
    });
    session.record_memory_injection(
        "summary".to_string(),
        "content".to_string(),
        1,
        5,
        Vec::new(),
    );
    session.record_replay_display_message("system", Some("Launch".to_string()), "boot");
    session.save()?;

    let stub = Session::load_startup_stub(session_id)?;
    assert_eq!(stub.id, session_id);
    assert_eq!(stub.parent_id.as_deref(), Some("parent_123"));
    assert_eq!(stub.title.as_deref(), Some("startup stub"));
    assert_eq!(stub.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(stub.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(stub.provider_key.as_deref(), Some("openai"));
    assert_eq!(stub.route_api_method.as_deref(), Some("openai-api"));
    assert!(stub.is_canary);
    assert!(stub.messages.is_empty());
    assert!(stub.env_snapshots.is_empty());
    assert!(stub.memory_injections.is_empty());
    assert!(stub.replay_events.is_empty());
    Ok(())
}
#[test]
fn load_for_remote_startup_preserves_messages_and_replay_but_skips_heavy_vectors() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-remote-startup-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let session_id = "session_remote_startup_roundtrip";
    let mut session = Session::create_with_id(
        session_id.to_string(),
        Some("parent_remote".to_string()),
        Some("remote startup".to_string()),
    );
    session.model = Some("gpt-5.4".to_string());
    session.reasoning_effort = Some("medium".to_string());
    session.append_stored_message(StoredMessage {
        id: "msg_remote_1".to_string(),
        role: Role::Assistant,
        content: vec![ContentBlock::Text {
            text: "hello remote startup".to_string(),
            cache_control: None,
        }],
        display_role: None,
        timestamp: Some(Utc::now()),
        tool_duration_ms: None,
        token_usage: None,
    });
    session.record_env_snapshot(EnvSnapshot {
        captured_at: Utc::now(),
        reason: "resume".to_string(),
        session_id: session_id.to_string(),
        working_dir: Some(temp_home.path().to_string_lossy().to_string()),
        provider: "openai".to_string(),
        model: "gpt-5.4".to_string(),
        jcode_version: "test".to_string(),
        jcode_git_hash: Some("abc123".to_string()),
        jcode_git_dirty: Some(false),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        pid: 123,
        is_selfdev: false,
        is_debug: false,
        is_canary: false,
        testing_build: None,
        working_git: None,
    });
    session.record_memory_injection(
        "summary".to_string(),
        "content".to_string(),
        1,
        5,
        Vec::new(),
    );
    session.record_replay_display_message("system", Some("Launch".to_string()), "boot");
    session.save()?;

    let loaded = Session::load_for_remote_startup(session_id)?;
    assert_eq!(loaded.id, session_id);
    assert_eq!(loaded.parent_id.as_deref(), Some("parent_remote"));
    assert_eq!(loaded.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(loaded.reasoning_effort.as_deref(), Some("medium"));
    assert_eq!(loaded.messages.len(), 1);
    assert!(loaded.replay_events.is_empty());
    assert!(loaded.env_snapshots.is_empty());
    assert!(loaded.memory_injections.is_empty());
    Ok(())
}
#[test]
fn test_create_marks_debug_when_test_session_env_enabled() {
    let _env_lock = lock_env();
    let _test_flag = EnvVarGuard::set("JCODE_TEST_SESSION", "1");

    let s1 = Session::create(None, None);
    assert!(s1.is_debug);

    let s2 = Session::create_with_id("session_test_1".to_string(), None, None);
    assert!(s2.is_debug);
}
#[test]
fn test_create_not_debug_when_test_session_env_disabled() {
    let _env_lock = lock_env();
    let _test_flag = EnvVarGuard::set("JCODE_TEST_SESSION", "0");

    let s = Session::create(None, None);
    assert!(!s.is_debug);
}
#[test]
fn test_recover_crashed_sessions_preserves_debug_flag() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-recover-debug-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());
    let _test_flag = EnvVarGuard::set("JCODE_TEST_SESSION", "0");

    let mut crashed = Session::create_with_id(
        "session_recover_debug_source".to_string(),
        None,
        Some("debug source".to_string()),
    );
    crashed.is_debug = true;
    crashed.mark_crashed(Some("test crash".to_string()));
    crashed.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "hello".to_string(),
            cache_control: None,
        }],
    );
    crashed.save()?;

    let recovered_ids = recover_crashed_sessions()?;
    assert_eq!(recovered_ids.len(), 1);

    let recovered = Session::load(&recovered_ids[0])?;
    assert!(recovered.is_debug);
    Ok(())
}
#[test]
fn test_recover_crashed_sessions_by_ids_restores_only_selected_group() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-recover-selected-crash-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());
    let _test_flag = EnvVarGuard::set("JCODE_TEST_SESSION", "0");

    let now = Utc::now();
    for (id, active_at) in [
        ("session_selected_crash", now),
        (
            "session_stale_unselected_crash",
            now - chrono::Duration::minutes(5),
        ),
    ] {
        let mut crashed = Session::create_with_id(id.to_string(), None, Some(id.to_string()));
        crashed.mark_crashed(Some("test crash".to_string()));
        crashed.add_message(
            Role::User,
            vec![ContentBlock::Text {
                text: format!("message from {id}"),
                cache_control: None,
            }],
        );
        crashed.last_active_at = Some(active_at);
        crashed.save()?;
    }

    let recovered_ids = recover_crashed_sessions_by_ids(&["session_selected_crash".to_string()])?;
    assert_eq!(recovered_ids.len(), 1);

    let recovered = Session::load(&recovered_ids[0])?;
    assert_eq!(
        recovered.parent_id.as_deref(),
        Some("session_selected_crash")
    );
    let stale = Session::load("session_stale_unselected_crash")?;
    assert!(matches!(stale.status, SessionStatus::Crashed { .. }));
    Ok(())
}
#[test]
fn untouched_session_is_not_persisted_until_real_conversation_starts() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-session-lazy-save-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let id = "session_untouched_lazy_save";
    let mut session = Session::create_with_id(id.to_string(), None, None);
    assert!(session.ensure_initial_session_context_message());
    session.save()?;
    assert!(!session_path(id)?.exists());

    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "hello".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;
    assert!(session_path(id)?.exists());
    Ok(())
}
#[test]
fn session_created_with_title_is_persisted_before_first_visible_message() -> Result<()> {
    // Regression for #1144: `Session::create(_, Some(title))` was skipped by
    // the untouched-session gate, so later lookups by id found nothing.
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-session-titled-save-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let id = "session_titled_eager_save";
    let mut session = Session::create_with_id(id.to_string(), None, Some("review".to_string()));
    assert!(session.ensure_initial_session_context_message());
    session.save()?;
    assert!(session_path(id)?.exists());

    let stub = Session::load_startup_stub(id)?;
    assert_eq!(stub.title.as_deref(), Some("review"));
    Ok(())
}
#[test]
fn test_save_persists_full_session_content() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-session-save-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let mut session = Session::create_with_id(
        "session_save_persist_test".to_string(),
        None,
        Some("save fidelity test".to_string()),
    );

    session.add_message(
        Role::User,
        vec![ContentBlock::ToolResult {
            tool_use_id: "tool_1".to_string(),
            content: "OPENROUTER_API_KEY=sk-or-v1-abcdefghijklmnopqrstuvwxyz0123456789".to_string(),
            is_error: None,
        }],
    );

    session.add_message(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: "tool_2".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({
                "command": "echo ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123"
            }),
            thought_signature: None,
        }],
    );

    session.save()?;

    let loaded = Session::load("session_save_persist_test")?;

    let ContentBlock::ToolResult { content, .. } = &loaded.messages[0].content[0] else {
        return Err(anyhow!("expected tool result block"));
    };
    assert!(content.contains("sk-or-v1-abcdefghijklmnopqrstuvwxyz0123456789"));
    assert!(!content.contains("[REDACTED_SECRET]"));

    let ContentBlock::ToolUse { input, .. } = &loaded.messages[1].content[0] else {
        return Err(anyhow!("expected tool use block"));
    };
    let input_str = input.to_string();
    assert!(input_str.contains("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123"));
    assert!(!input_str.contains("[REDACTED_SECRET]"));
    Ok(())
}
#[test]
fn test_save_persists_compaction_state() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-session-compaction-save-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let mut session = Session::create_with_id(
        "session_compaction_persist_test".to_string(),
        None,
        Some("compaction persistence test".to_string()),
    );
    session.compaction = Some(StoredCompactionState {
        summary_text: "saved summary".to_string(),
        openai_encrypted_content: None,
        covers_up_to_turn: 8,
        original_turn_count: 8,
        compacted_count: 8,
    });

    session.save()?;

    let loaded = Session::load("session_compaction_persist_test")?;
    assert_eq!(loaded.compaction, session.compaction);
    Ok(())
}
#[test]
fn test_save_persists_provider_key() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-session-provider-key-save-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let mut session = Session::create_with_id(
        "session_provider_key_persist_test".to_string(),
        None,
        Some("provider key persistence test".to_string()),
    );
    session.provider_key = Some("opencode".to_string());
    session.model = Some("anthropic/claude-sonnet-4".to_string());

    session.save()?;

    let loaded = Session::load("session_provider_key_persist_test")?;
    assert_eq!(loaded.provider_key.as_deref(), Some("opencode"));
    assert_eq!(loaded.model.as_deref(), Some("anthropic/claude-sonnet-4"));
    Ok(())
}
#[test]
fn test_save_persists_reasoning_effort() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-session-reasoning-effort-save-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let mut session = Session::create_with_id(
        "session_reasoning_effort_persist_test".to_string(),
        None,
        Some("reasoning effort persistence test".to_string()),
    );
    session.model = Some("gpt-5.4".to_string());
    session.reasoning_effort = Some("xhigh".to_string());

    session.save()?;

    let loaded = Session::load("session_reasoning_effort_persist_test")?;
    assert_eq!(loaded.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(loaded.reasoning_effort.as_deref(), Some("xhigh"));
    Ok(())
}
#[test]
fn test_save_appends_journal_and_load_replays_it() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-session-journal-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let mut session = Session::create_with_id(
        "session_journal_append_test".to_string(),
        None,
        Some("journal append test".to_string()),
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "first".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;

    let snapshot_path = session_path("session_journal_append_test")?;
    let journal_path = session_journal_path("session_journal_append_test")?;
    assert!(snapshot_path.exists());
    assert!(!journal_path.exists());

    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "second".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;

    assert!(journal_path.exists());
    let journal = std::fs::read_to_string(&journal_path)?;
    assert!(journal.contains("second"));

    let loaded = Session::load("session_journal_append_test")?;
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[1].content_preview(), "second");
    Ok(())
}
#[test]
fn test_save_checkpoints_after_full_mutation_and_clears_journal() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-session-checkpoint-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let mut session = Session::create_with_id(
        "session_journal_checkpoint_test".to_string(),
        None,
        Some("checkpoint test".to_string()),
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "one".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;

    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "two".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;

    let journal_path = session_journal_path("session_journal_checkpoint_test")?;
    assert!(journal_path.exists());

    session.truncate_messages(1);
    session.title = Some("checkpointed title".to_string());
    session.save()?;

    assert!(!journal_path.exists());

    let loaded = Session::load("session_journal_checkpoint_test")?;
    assert_eq!(loaded.title.as_deref(), Some("checkpointed title"));
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].content_preview(), "one");
    Ok(())
}
#[test]
fn test_journal_replay_skips_corrupt_line_and_keeps_tail() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-session-journal-corrupt-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let session_id = "session_journal_corrupt_tail_test";
    let mut session = Session::create_with_id(
        session_id.to_string(),
        None,
        Some("corrupt journal test".to_string()),
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "first".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;

    // Two good journal entries.
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "second".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "third".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;

    // Corrupt the middle line (torn write) but keep the final line intact.
    let journal_path = session_journal_path(session_id)?;
    let journal = std::fs::read_to_string(&journal_path)?;
    let lines: Vec<&str> = journal.lines().collect();
    assert_eq!(lines.len(), 2);
    let torn = &lines[0][..lines[0].len() / 2];
    std::fs::write(&journal_path, format!("{}\n{}\n", torn, lines[1]))?;

    // The last prompt ("third") must survive even though an earlier journal
    // line is unparseable.
    let loaded = Session::load(session_id)?;
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].content_preview(), "first");
    assert_eq!(loaded.messages[1].content_preview(), "third");

    let remote = Session::load_for_remote_startup(session_id)?;
    assert_eq!(remote.messages.len(), 2);
    assert_eq!(remote.messages[1].content_preview(), "third");
    Ok(())
}
#[test]
fn test_journal_replay_salvages_glued_entries_on_torn_line() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-session-journal-glued-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let session_id = "session_journal_glued_test";
    let mut session = Session::create_with_id(
        session_id.to_string(),
        None,
        Some("glued journal test".to_string()),
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "first".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "second".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "third".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;

    // Simulate a torn append glued to the next complete entry: half of entry 1
    // followed (no newline) by all of entry 2 on the same line.
    let journal_path = session_journal_path(session_id)?;
    let journal = std::fs::read_to_string(&journal_path)?;
    let lines: Vec<&str> = journal.lines().collect();
    assert_eq!(lines.len(), 2);
    let torn = &lines[0][..lines[0].len() / 2];
    std::fs::write(&journal_path, format!("{}{}\n", torn, lines[1]))?;

    // The glued complete entry ("third") must be salvaged from the corrupt line.
    let loaded = Session::load(session_id)?;
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].content_preview(), "first");
    assert_eq!(loaded.messages[1].content_preview(), "third");
    Ok(())
}
#[test]
fn test_corrupt_journal_heals_via_checkpoint_on_next_save() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-session-journal-heal-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let session_id = "session_journal_heal_test";
    let mut session = Session::create_with_id(
        session_id.to_string(),
        None,
        Some("heal journal test".to_string()),
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "first".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "second".to_string(),
            cache_control: None,
        }],
    );
    session.save()?;

    // Corrupt the only journal line.
    let journal_path = session_journal_path(session_id)?;
    let journal = std::fs::read_to_string(&journal_path)?;
    let line = journal.lines().next().unwrap_or_default();
    std::fs::write(&journal_path, &line[..line.len() / 2])?;

    let mut loaded = Session::load(session_id)?;
    assert_eq!(loaded.messages.len(), 1);

    // A forensic backup of the corrupt journal is kept.
    let backup_path = journal_path.with_extension("corrupt.jsonl");
    assert!(backup_path.exists());

    // The next save checkpoints a full snapshot and removes the corrupt journal,
    // so the bad line is never replayed again.
    loaded.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "after heal".to_string(),
            cache_control: None,
        }],
    );
    loaded.save()?;
    assert!(!journal_path.exists());

    let reloaded = Session::load(session_id)?;
    assert_eq!(reloaded.messages.len(), 2);
    assert_eq!(reloaded.messages[1].content_preview(), "after heal");
    Ok(())
}
