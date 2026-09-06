#[test]
fn resolve_swarm_spawn_model_prefers_configured_model_over_coordinator_model() {
    let selection = resolve_swarm_spawn_selection(
        None,
        Some("openai/gpt-5.4@OpenAI".to_string()),
        &coordinator_identity(
            Some("nvidia/llama-3.3-nemotron-super-49b-v1"),
            Some("nvidia"),
            Some("openai-compatible:nvidia-nim"),
        ),
    );

    assert_eq!(selection.model.as_deref(), Some("openai/gpt-5.4@OpenAI"));
    assert_eq!(selection.provider_key.as_deref(), Some("openrouter"));
    // A different configured model must not inherit the coordinator's route.
    assert_eq!(selection.route_api_method, None);
}

#[test]
fn resolve_swarm_spawn_model_inherits_coordinator_when_unconfigured() {
    let selection = resolve_swarm_spawn_selection(
        None,
        None,
        &coordinator_identity(
            Some("nvidia/llama-3.3-nemotron-super-49b-v1"),
            Some("nvidia"),
            Some("openai-compatible:nvidia-nim"),
        ),
    );

    assert_eq!(
        selection.model.as_deref(),
        Some("nvidia/llama-3.3-nemotron-super-49b-v1")
    );
    assert_eq!(selection.provider_key.as_deref(), Some("nvidia"));
    assert_eq!(
        selection.route_api_method.as_deref(),
        Some("openai-compatible:nvidia-nim")
    );
}

#[test]
fn resolve_swarm_spawn_model_inherits_coordinator_auth_route_for_oauth_vs_api() {
    // Regression: a coordinator on the Claude API route must spawn agents on
    // the same API route, not Claude OAuth (the config default).
    let selection = resolve_swarm_spawn_selection(
        None,
        None,
        &coordinator_identity(
            Some("claude-opus-4-6"),
            Some("claude-api"),
            Some("claude-api"),
        ),
    );

    assert_eq!(selection.model.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(selection.provider_key.as_deref(), Some("claude-api"));
    assert_eq!(selection.route_api_method.as_deref(), Some("claude-api"));
}

#[test]
fn resolve_swarm_spawn_model_keeps_provider_key_when_config_matches_coordinator() {
    let selection = resolve_swarm_spawn_selection(
        None,
        Some("custom-model".to_string()),
        &coordinator_identity(
            Some("custom-model"),
            Some("custom-provider"),
            Some("custom-route"),
        ),
    );

    assert_eq!(selection.model.as_deref(), Some("custom-model"));
    assert_eq!(selection.provider_key.as_deref(), Some("custom-provider"));
    assert_eq!(selection.route_api_method.as_deref(), Some("custom-route"));
}

#[test]
fn resolve_swarm_spawn_model_openai_api_prefix_pins_api_route_over_coordinator() {
    // `agents.swarm_model = "openai-api:gpt-5.5"` must spawn agents on GPT-5.5
    // via the OpenAI API key route, regardless of the coordinator's model/auth.
    let selection = resolve_swarm_spawn_selection(
        None,
        Some("openai-api:gpt-5.5".to_string()),
        &coordinator_identity(
            Some("claude-opus-4-8"),
            Some("claude-oauth"),
            Some("claude-oauth"),
        ),
    );

    assert_eq!(selection.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(selection.provider_key.as_deref(), Some("openai-api-key"));
    assert_eq!(
        selection.route_api_method.as_deref(),
        Some("openai-api-key")
    );
}

#[test]
fn resolve_swarm_spawn_model_auth_route_prefixes_pin_expected_routes() {
    for (configured, expected_model, expected_key) in [
        ("openai-api:gpt-5.5", "gpt-5.5", "openai-api-key"),
        ("openai-oauth:gpt-5.5", "gpt-5.5", "openai-oauth"),
        (
            "claude-api:claude-opus-4-8",
            "claude-opus-4-8",
            "anthropic-api-key",
        ),
        (
            "claude-oauth:claude-opus-4-8",
            "claude-opus-4-8",
            "claude-oauth",
        ),
    ] {
        let selection = resolve_swarm_spawn_selection(
            None,
            Some(configured.to_string()),
            &coordinator_identity(
                Some("some-other-model"),
                Some("some-key"),
                Some("some-route"),
            ),
        );
        assert_eq!(
            selection.model.as_deref(),
            Some(expected_model),
            "configured {configured:?} model",
        );
        assert_eq!(
            selection.provider_key.as_deref(),
            Some(expected_key),
            "configured {configured:?} provider_key",
        );
        assert_eq!(
            selection.route_api_method.as_deref(),
            Some(expected_key),
            "configured {configured:?} route_api_method",
        );
    }
}

#[test]
fn resolve_swarm_spawn_model_inherit_sentinel_uses_coordinator_model() {
    for sentinel in ["inherit", "INHERIT", "coordinator", " inherit ", ""] {
        let selection = resolve_swarm_spawn_selection(
            None,
            Some(sentinel.to_string()),
            &coordinator_identity(
                Some("nvidia/llama-3.3-nemotron-super-49b-v1"),
                Some("nvidia"),
                Some("openai-compatible:nvidia-nim"),
            ),
        );

        assert_eq!(
            selection.model.as_deref(),
            Some("nvidia/llama-3.3-nemotron-super-49b-v1"),
            "sentinel {sentinel:?} should inherit coordinator model",
        );
        assert_eq!(
            selection.provider_key.as_deref(),
            Some("nvidia"),
            "sentinel {sentinel:?} should inherit coordinator provider key",
        );
        assert_eq!(
            selection.route_api_method.as_deref(),
            Some("openai-compatible:nvidia-nim"),
            "sentinel {sentinel:?} should inherit coordinator auth route",
        );
    }
}

#[test]
fn resolve_swarm_spawn_model_requested_model_overrides_configured_pin() {
    for requested in ["openai-api:gpt-5.5", "  openai-api:gpt-5.5 \t"] {
        let selection = resolve_swarm_spawn_selection(
            Some(requested.to_string()),
            Some("claude-oauth:claude-opus-4-8".to_string()),
            &coordinator_identity(
                Some("claude-fable-5"),
                Some("claude-oauth"),
                Some("claude-oauth"),
            ),
        );

        assert_eq!(selection.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(selection.provider_key.as_deref(), Some("openai-api-key"));
        assert_eq!(
            selection.route_api_method.as_deref(),
            Some("openai-api-key")
        );
    }
}

#[test]
fn resolve_swarm_spawn_model_requested_inherit_overrides_configured_pin() {
    for requested in [
        "inherit",
        "INHERIT",
        "coordinator",
        " COORDINATOR ",
        " inherit ",
    ] {
        let selection = resolve_swarm_spawn_selection(
            Some(requested.to_string()),
            Some("openai-api:gpt-5.5".to_string()),
            &coordinator_identity(
                Some("claude-fable-5"),
                Some("claude-api"),
                Some("claude-api"),
            ),
        );

        assert_eq!(selection.model.as_deref(), Some("claude-fable-5"));
        assert_eq!(selection.provider_key.as_deref(), Some("claude-api"));
        assert_eq!(selection.route_api_method.as_deref(), Some("claude-api"));
    }
}

#[test]
fn resolve_swarm_spawn_model_requested_matching_coordinator_model_keeps_route() {
    let selection = resolve_swarm_spawn_selection(
        Some(" custom-model ".to_string()),
        Some("openai-api:gpt-5.5".to_string()),
        &coordinator_identity(
            Some("custom-model"),
            Some("custom-provider"),
            Some("custom-route"),
        ),
    );

    assert_eq!(selection.model.as_deref(), Some("custom-model"));
    assert_eq!(selection.provider_key.as_deref(), Some("custom-provider"));
    assert_eq!(selection.route_api_method.as_deref(), Some("custom-route"));
}

#[test]
fn resolve_swarm_spawn_model_blank_requested_model_falls_back_to_config() {
    for requested in ["", "   ", "\t\n"] {
        let selection = resolve_swarm_spawn_selection(
            Some(requested.to_string()),
            Some("openai-api:gpt-5.5".to_string()),
            &coordinator_identity(
                Some("claude-fable-5"),
                Some("claude-oauth"),
                Some("claude-oauth"),
            ),
        );

        assert_eq!(selection.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(selection.provider_key.as_deref(), Some("openai-api-key"));
        assert_eq!(
            selection.route_api_method.as_deref(),
            Some("openai-api-key")
        );
    }
}

#[test]
fn resolve_swarm_spawn_model_omitted_request_trims_configured_model() {
    let selection = resolve_swarm_spawn_selection(
        None,
        Some(" \topenai-api:gpt-5.5 \n".to_string()),
        &coordinator_identity(
            Some("claude-fable-5"),
            Some("claude-oauth"),
            Some("claude-oauth"),
        ),
    );

    assert_eq!(selection.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(selection.provider_key.as_deref(), Some("openai-api-key"));
    assert_eq!(
        selection.route_api_method.as_deref(),
        Some("openai-api-key")
    );
}

#[test]
fn resolve_swarm_spawn_model_blank_requested_model_inherits_when_unconfigured() {
    let selection = resolve_swarm_spawn_selection(
        Some(" \t\n".to_string()),
        None,
        &coordinator_identity(
            Some("custom-model"),
            Some("custom-provider"),
            Some("custom-route"),
        ),
    );

    assert_eq!(selection.model.as_deref(), Some("custom-model"));
    assert_eq!(selection.provider_key.as_deref(), Some("custom-provider"));
    assert_eq!(selection.route_api_method.as_deref(), Some("custom-route"));
}

#[tokio::test]
async fn coordinator_identity_uses_live_agent_when_lock_is_available() {
    let agent = test_agent_with_working_dir("coord", "/tmp/coord").await;
    let live_model = agent.lock().await.provider_model();
    let sessions = Arc::new(RwLock::new(HashMap::new()));
    sessions
        .write()
        .await
        .insert("coord".to_string(), Arc::clone(&agent));

    let identity = resolve_coordinator_spawn_identity("coord", &sessions).await;
    assert_eq!(identity.model.as_deref(), Some(live_model.as_str()));
}

#[tokio::test]
async fn coordinator_identity_falls_back_to_persisted_session_when_agent_busy() {
    let _guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    crate::env::set_var("JCODE_HOME", temp_home.path());

    let agent = test_agent_with_working_dir("coord_busy", "/tmp/coord").await;

    // Persist a coordinator session that records a concrete model + auth route.
    // Persist after the agent is built so it reflects the authoritative on-disk
    // snapshot the spawn path will read when the agent lock is unavailable.
    let mut session = crate::session::Session::create_with_id("coord_busy".to_string(), None, None);
    session.model = Some("claude-opus-4-6".to_string());
    session.provider_key = Some("claude-api".to_string());
    session.route_api_method = Some("claude-api".to_string());
    session.subagent_model = Some("claude-oauth:claude-opus-4-6".to_string());
    session.reasoning_effort = Some("high".to_string());
    session.add_message(
        crate::message::Role::User,
        vec![crate::message::ContentBlock::Text {
            text: "Inspect the worker's selected route before continuing.".to_string(),
            cache_control: None,
        }],
    );
    session.save().expect("persist coordinator session");

    // Hold the agent lock to simulate a coordinator mid-turn: the spawn path
    // must not block and must read the persisted identity instead of defaults.
    let _held = agent.lock().await;
    let sessions = Arc::new(RwLock::new(HashMap::new()));
    sessions
        .write()
        .await
        .insert("coord_busy".to_string(), Arc::clone(&agent));

    let identity = resolve_coordinator_spawn_identity("coord_busy", &sessions).await;
    assert_eq!(identity.model.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(identity.provider_key.as_deref(), Some("claude-api"));
    assert_eq!(identity.route_api_method.as_deref(), Some("claude-api"));
    assert_eq!(
        identity.subagent_model.as_deref(),
        Some("claude-oauth:claude-opus-4-6")
    );
    assert_eq!(identity.reasoning_effort.as_deref(), Some("high"));

    crate::env::remove_var("JCODE_HOME");
}

#[tokio::test]
async fn spawn_bootstraps_coordinator_when_swarm_has_none() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["req".to_string()]),
    )])));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::new()));
    let swarm_plans = Arc::new(RwLock::new(HashMap::<String, VersionedPlan>::new()));
    let (req_member, _req_rx) = member("req", Some("swarm-1"), "agent");
    swarm_members
        .write()
        .await
        .insert("req".to_string(), req_member);
    let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel();

    let swarm_id = ensure_spawn_coordinator_swarm(
        1,
        "req",
        &client_event_tx,
        &swarm_members,
        &swarms_by_id,
        &swarm_coordinators,
        &swarm_plans,
        32,
    )
    .await;

    assert_eq!(swarm_id.as_deref(), Some("swarm-1"));
    assert_eq!(
        swarm_coordinators
            .read()
            .await
            .get("swarm-1")
            .map(String::as_str),
        Some("req")
    );
    assert_eq!(
        swarm_members
            .read()
            .await
            .get("req")
            .map(|member| member.role.as_str()),
        Some("coordinator")
    );
    assert!(matches!(
        client_event_rx.recv().await,
        Some(ServerEvent::Notification {
            notification_type: NotificationType::Message { .. },
            message,
            ..
        }) if message == "You are the coordinator for this swarm."
    ));
}

#[tokio::test]
async fn nested_agent_cannot_spawn_when_root_is_light_or_normal() {
    // Both explicit light-swarm effort and ordinary ad hoc swarm use are
    // one-level fan-out. A spawned child cannot grow another generation.
    for (root_id, effort) in [
        ("light-root-no-recursion", Some("swarm")),
        ("normal-root-no-recursion", None),
    ] {
        crate::session_effort::forget_session_effort(root_id);
        crate::session_effort::record_session_effort(root_id, effort);
        let swarm_id = format!("swarm-{root_id}");
        let child_id = format!("child-{root_id}");
        let swarm_members = Arc::new(RwLock::new(HashMap::new()));
        let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
            swarm_id.clone(),
            HashSet::from([child_id.clone(), root_id.to_string()]),
        )])));
        let swarm_coordinators = Arc::new(RwLock::new(HashMap::from([(
            swarm_id.clone(),
            root_id.to_string(),
        )])));
        let swarm_plans = Arc::new(RwLock::new(HashMap::<String, VersionedPlan>::new()));
        let (mut child_member, _child_rx) = member(&child_id, Some(&swarm_id), "agent");
        child_member.report_back_to_session_id = Some(root_id.to_string());
        let (root_member, _root_rx) = member(root_id, Some(&swarm_id), "coordinator");
        let mut members = swarm_members.write().await;
        members.insert(child_id.clone(), child_member);
        members.insert(root_id.to_string(), root_member);
        drop(members);
        let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel();

        let refused = ensure_spawn_coordinator_swarm(
            2,
            &child_id,
            &client_event_tx,
            &swarm_members,
            &swarms_by_id,
            &swarm_coordinators,
            &swarm_plans,
            32,
        )
        .await;

        crate::session_effort::forget_session_effort(root_id);
        assert!(refused.is_none());
        assert_eq!(
            swarm_coordinators
                .read()
                .await
                .get(&swarm_id)
                .map(String::as_str),
            Some(root_id)
        );
        assert_eq!(
            swarm_members
                .read()
                .await
                .get(&child_id)
                .map(|member| member.role.as_str()),
            Some("agent")
        );
        assert!(matches!(
            client_event_rx.recv().await,
            Some(ServerEvent::Error { message, .. })
                if message.contains("Recursive swarm spawning is disabled")
                    && message.contains(&format!("Only the root session ({root_id}) may spawn agents"))
        ));
    }
}

#[tokio::test]
async fn nested_agent_can_spawn_when_root_is_deep() {
    let root_id = "deep-root-recursive";
    crate::session_effort::record_session_effort(root_id, Some("swarm-deep"));

    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-deep".to_string(),
        HashSet::from(["deep-child".to_string(), root_id.to_string()]),
    )])));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::from([(
        "swarm-deep".to_string(),
        root_id.to_string(),
    )])));
    let swarm_plans = Arc::new(RwLock::new(HashMap::<String, VersionedPlan>::new()));
    let (mut child_member, _child_rx) = member("deep-child", Some("swarm-deep"), "agent");
    child_member.report_back_to_session_id = Some(root_id.to_string());
    let (root_member, _root_rx) = member(root_id, Some("swarm-deep"), "coordinator");
    let mut members = swarm_members.write().await;
    members.insert("deep-child".to_string(), child_member);
    members.insert(root_id.to_string(), root_member);
    drop(members);
    let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel();

    let allowed = ensure_spawn_coordinator_swarm(
        3,
        "deep-child",
        &client_event_tx,
        &swarm_members,
        &swarms_by_id,
        &swarm_coordinators,
        &swarm_plans,
        32,
    )
    .await;

    crate::session_effort::forget_session_effort(root_id);
    assert_eq!(allowed.as_deref(), Some("swarm-deep"));
    assert!(client_event_rx.try_recv().is_err());
}

#[tokio::test]
async fn spawn_allowed_at_arbitrary_depth_without_depth_cap() {
    // Deep-swarm mode still allows recursive decomposition at arbitrary depth.
    let root_id = "deep-root-arbitrary-depth";
    crate::session_effort::record_session_effort(root_id, Some("swarm-deep"));
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        root_id.to_string(),
    )])));
    let swarm_plans = Arc::new(RwLock::new(HashMap::<String, VersionedPlan>::new()));
    {
        let mut members = swarm_members.write().await;
        let (root, _rx) = member(root_id, Some("swarm-1"), "coordinator");
        members.insert(root_id.to_string(), root);
        let chain = [
            ("a", root_id),
            ("b", "a"),
            ("c", "b"),
            ("d", "c"),
            ("e", "d"),
            ("f", "e"),
        ];
        for (id, parent) in chain {
            let (mut m, _rx) = member(id, Some("swarm-1"), "agent");
            m.report_back_to_session_id = Some(parent.to_string());
            members.insert(id.to_string(), m);
        }
    }
    let (client_event_tx, _client_event_rx) = mpsc::unbounded_channel();

    // `f` is deeply nested but the swarm is far below the member cap, so spawning
    // is allowed.
    let allowed = ensure_spawn_coordinator_swarm(
        7,
        "f",
        &client_event_tx,
        &swarm_members,
        &swarms_by_id,
        &swarm_coordinators,
        &swarm_plans,
        32,
    )
    .await;
    crate::session_effort::forget_session_effort(root_id);
    assert_eq!(allowed.as_deref(), Some("swarm-1"));
}

#[tokio::test]
async fn spawn_rejected_when_member_limit_reached() {
    use crate::server::swarm::MAX_SWARM_MEMBERS;

    // Fill the swarm to the member cap; the next spawn must be refused.
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        "root".to_string(),
    )])));
    let swarm_plans = Arc::new(RwLock::new(HashMap::<String, VersionedPlan>::new()));
    {
        let mut members = swarm_members.write().await;
        let (root, _rx) = member("root", Some("swarm-1"), "coordinator");
        members.insert("root".to_string(), root);
        // Add filler members so the swarm holds exactly MAX_SWARM_MEMBERS total.
        for idx in 1..MAX_SWARM_MEMBERS {
            let id = format!("agent-{idx}");
            let (mut m, _rx) = member(&id, Some("swarm-1"), "agent");
            m.report_back_to_session_id = Some("root".to_string());
            members.insert(id, m);
        }
    }
    let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel();

    let refused = ensure_spawn_coordinator_swarm(
        7,
        "root",
        &client_event_tx,
        &swarm_members,
        &swarms_by_id,
        &swarm_coordinators,
        &swarm_plans,
        0,
    )
    .await;
    assert!(refused.is_none());
    assert!(matches!(
        client_event_rx.recv().await,
        Some(ServerEvent::Error { message, .. })
            if message.contains("Swarm member limit reached")
    ));
}

#[tokio::test]
async fn terminal_members_do_not_consume_spawn_capacity() {
    use crate::server::swarm::MAX_SWARM_MEMBERS;

    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        "root".to_string(),
    )])));
    let swarm_plans = Arc::new(RwLock::new(HashMap::<String, VersionedPlan>::new()));
    {
        let mut members = swarm_members.write().await;
        let (root, _rx) = member("root", Some("swarm-1"), "coordinator");
        members.insert("root".to_string(), root);
        for idx in 0..MAX_SWARM_MEMBERS {
            let id = format!("historical-{idx}");
            let (mut historical, _rx) = member(&id, Some("swarm-1"), "agent");
            historical.status = if idx % 2 == 0 {
                "completed".to_string()
            } else {
                "stopped".to_string()
            };
            historical.latest_completion_report = Some(format!("report {idx}"));
            historical.report_back_to_session_id = Some("root".to_string());
            members.insert(id, historical);
        }
    }
    let (client_event_tx, _client_event_rx) = mpsc::unbounded_channel();

    let allowed = ensure_spawn_coordinator_swarm(
        7,
        "root",
        &client_event_tx,
        &swarm_members,
        &swarms_by_id,
        &swarm_coordinators,
        &swarm_plans,
        32,
    )
    .await;

    assert_eq!(allowed.as_deref(), Some("swarm-1"));
}

#[tokio::test]
async fn spawn_rejected_at_configured_live_agent_limit() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        "root".to_string(),
    )])));
    let swarm_plans = Arc::new(RwLock::new(HashMap::<String, VersionedPlan>::new()));
    {
        let mut members = swarm_members.write().await;
        let (root, _rx) = member("root", Some("swarm-1"), "coordinator");
        members.insert("root".to_string(), root);
        for idx in 0..2 {
            let id = format!("agent-{idx}");
            let (mut worker, _rx) = member(&id, Some("swarm-1"), "agent");
            worker.report_back_to_session_id = Some("root".to_string());
            members.insert(id, worker);
        }
    }
    let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel();

    let refused = ensure_spawn_coordinator_swarm(
        7,
        "root",
        &client_event_tx,
        &swarm_members,
        &swarms_by_id,
        &swarm_coordinators,
        &swarm_plans,
        2,
    )
    .await;

    assert!(refused.is_none());
    assert!(matches!(
        client_event_rx.recv().await,
        Some(ServerEvent::Error { message, .. })
            if message.contains("Swarm live-agent limit reached (max 2")
    ));
}

#[tokio::test]
async fn spawn_admission_lock_serializes_per_swarm_only() {
    use std::time::Duration;

    let key = format!("lock-test-{}", std::process::id());
    let same_a = spawn_admission_lock(&key);
    let same_b = spawn_admission_lock(&key);
    let other = spawn_admission_lock(&format!("{key}-other"));

    let held = same_a.lock().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), same_b.lock())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), other.lock())
            .await
            .is_ok()
    );
    drop(held);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), same_b.lock())
            .await
            .is_ok()
    );
}

