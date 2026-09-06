use super::*;
use anyhow::{Result, anyhow};

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        crate::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(value) = &self.previous {
            crate::env::set_var(self.key, value);
        } else {
            crate::env::remove_var(self.key);
        }
    }
}

#[tokio::test]
async fn handle_clear_session_replaces_runtime_handles_and_updates_shutdown_registration()
-> Result<()> {
    check_clear_session(false, false, false).await
}

#[tokio::test]
async fn handle_clear_session_persists_debug_replacement_before_done() -> Result<()> {
    check_clear_session(true, false, false).await
}

#[tokio::test]
async fn handle_clear_session_persists_selfdev_replacement_before_done() -> Result<()> {
    check_clear_session(false, true, false).await
}

#[tokio::test]
async fn handle_clear_session_preserves_old_session_when_handoff_fails() -> Result<()> {
    check_clear_session(true, false, true).await
}

async fn check_clear_session(
    debug_session: bool,
    selfdev_client: bool,
    fail_persistence: bool,
) -> Result<()> {
    let _guard = crate::storage::lock_test_env();
    let home = tempfile::tempdir()?;
    let _home = EnvVarGuard::set("JCODE_HOME", home.path());
    let _runtime = EnvVarGuard::set("JCODE_RUNTIME_DIR", home.path().join("runtime"));
    let _test_session = EnvVarGuard::set("JCODE_TEST_SESSION", "0");

    let old_session_id = "session_before_clear";
    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    let registry = Registry::new(provider.clone()).await;
    let agent = Arc::new(Mutex::new(build_test_agent_with_id(
        provider.clone(),
        registry.clone(),
        old_session_id,
        Vec::new(),
    )));
    agent.lock().await.set_debug(debug_session);

    let old_queue = {
        let guard = agent.lock().await;
        guard.soft_interrupt_queue()
    };
    let old_background_signal = {
        let guard = agent.lock().await;
        guard.background_tool_signal()
    };
    let old_cancel_signal = {
        let guard = agent.lock().await;
        guard.graceful_shutdown_signal()
    };

    let sessions = Arc::new(RwLock::new(HashMap::from([(
        old_session_id.to_string(),
        Arc::clone(&agent),
    )])));
    let shutdown_signals = Arc::new(RwLock::new(HashMap::from([(
        old_session_id.to_string(),
        old_cancel_signal.clone(),
    )])));
    let soft_interrupt_queues: SessionInterruptQueues = Arc::new(RwLock::new(HashMap::from([(
        old_session_id.to_string(),
        old_queue.clone(),
    )])));
    let now = Instant::now();
    let client_connections = Arc::new(RwLock::new(HashMap::from([(
        "conn_clear".to_string(),
        ClientConnectionInfo {
            client_id: "conn_clear".to_string(),
            session_id: old_session_id.to_string(),
            client_instance_id: None,
            debug_client_id: Some("debug_clear".to_string()),
            connected_at: now,
            last_seen: now,
            is_processing: false,
            current_tool_name: None,
            terminal_env: Vec::new(),
            disconnect_tx: mpsc::unbounded_channel().0,
        },
    )])));
    let swarm_members = Arc::new(RwLock::new(HashMap::from([(
        old_session_id.to_string(),
        test_swarm_member(old_session_id, "ready"),
    )])));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-test".to_string(),
        HashSet::from([old_session_id.to_string()]),
    )])));
    let file_touch = FileTouchService::new();
    let channel_subscriptions = Arc::new(RwLock::new(HashMap::<
        String,
        HashMap<String, HashSet<String>>,
    >::new()));
    let channel_subscriptions_by_session = Arc::new(RwLock::new(HashMap::<
        String,
        HashMap<String, HashSet<String>>,
    >::new()));
    let swarm_plans = Arc::new(RwLock::new(HashMap::from([(
        "swarm-test".to_string(),
        VersionedPlan {
            items: Vec::new(),
            version: 1,
            participants: HashSet::from([old_session_id.to_string()]),
            task_progress: HashMap::new(),
            mode: "deep".to_string(),
            node_meta: HashMap::new(),
        },
    )])));
    let event_history = Arc::new(RwLock::new(VecDeque::<SwarmEvent>::new()));
    let event_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (swarm_event_tx, _swarm_event_rx) = broadcast::channel::<SwarmEvent>(8);
    let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel::<ServerEvent>();

    if fail_persistence {
        let sessions_path = home.path().join("sessions");
        if sessions_path.exists() {
            std::fs::remove_dir(&sessions_path)?;
        }
        std::fs::write(sessions_path, "a file cannot contain session snapshots")?;
    }
    let mut client_session_id = old_session_id.to_string();
    handle_clear_session(
        7,
        selfdev_client,
        &mut client_session_id,
        "conn_clear",
        &agent,
        &provider,
        &registry,
        &sessions,
        &shutdown_signals,
        &soft_interrupt_queues,
        &client_connections,
        &swarm_members,
        &swarms_by_id,
        &file_touch,
        &channel_subscriptions,
        &channel_subscriptions_by_session,
        &swarm_plans,
        &event_history,
        &event_counter,
        &swarm_event_tx,
        &client_event_tx,
    )
    .await;

    if fail_persistence {
        assert_eq!(client_session_id, old_session_id);
        assert_eq!(agent.lock().await.session_id(), old_session_id);
        assert!(crate::session::active_session_ids().contains(&client_session_id));
        let sessions = sessions.read().await;
        assert_eq!(sessions.len(), 1);
        assert!(Arc::ptr_eq(&sessions[old_session_id], &agent));
        assert!(Arc::ptr_eq(
            &soft_interrupt_queues.read().await[old_session_id],
            &old_queue,
        ));
        assert_eq!(
            client_connections.read().await["conn_clear"].session_id,
            old_session_id
        );
        assert!(swarm_members.read().await.contains_key(old_session_id));
        assert!(
            swarm_plans.read().await["swarm-test"]
                .participants
                .contains(old_session_id)
        );
        assert!(
            matches!(client_event_rx.try_recv()?, ServerEvent::Error { id: 7, message, .. }
            if message.contains("failed to persist replacement"))
        );
        assert!(
            client_event_rx.try_recv().is_err(),
            "a failed clear must not publish SessionId or Done"
        );
        return Ok(());
    }

    assert_ne!(client_session_id, old_session_id);
    if debug_session || selfdev_client {
        let stored = crate::session::Session::load(&client_session_id)?;
        assert_eq!(stored.is_debug, debug_session);
        assert_eq!(stored.is_canary, selfdev_client);
        assert!(!stored.saved);
        assert!(stored.title.is_none());
        assert!(stored.parent_id.is_none());
    } else {
        assert!(!crate::session::session_path(&client_session_id)?.exists());
    }
    let members = swarm_members.read().await;
    assert!(members.get(old_session_id).is_none());
    let replacement_member = members
        .get(&client_session_id)
        .expect("replacement session should remain registered for swarm tools");
    assert!(replacement_member.swarm_enabled);
    assert_eq!(replacement_member.status, "ready");
    assert_ne!(replacement_member.swarm_id.as_deref(), Some("swarm-test"));
    let replacement_swarm_id = replacement_member
        .swarm_id
        .clone()
        .expect("replacement session should get a fresh swarm identity");
    drop(members);
    assert!(swarms_by_id.read().await.get("swarm-test").is_none());
    assert!(
        swarms_by_id
            .read()
            .await
            .get(&replacement_swarm_id)
            .is_some_and(|sessions| sessions.contains(&client_session_id))
    );
    let plans = swarm_plans.read().await;
    assert!(!plans["swarm-test"].participants.contains(old_session_id));
    assert!(
        !plans["swarm-test"]
            .participants
            .contains(&client_session_id)
    );
    drop(plans);

    old_queue
        .lock()
        .map_err(|_| anyhow!("old queue lock"))?
        .push(jcode_agent_runtime::SoftInterruptMessage {
            content: "stale queued message".to_string(),
            images: Vec::new(),
            urgent: false,
            source: jcode_agent_runtime::SoftInterruptSource::User,
        });
    old_background_signal.fire();
    old_cancel_signal.fire();

    let (new_queue, new_background_signal, new_cancel_signal) = {
        let guard = agent.lock().await;
        (
            guard.soft_interrupt_queue(),
            guard.background_tool_signal(),
            guard.graceful_shutdown_signal(),
        )
    };

    assert!(!Arc::ptr_eq(&old_queue, &new_queue));
    assert!(!new_background_signal.is_set());
    assert!(!new_cancel_signal.is_set());
    assert!(!agent.lock().await.has_soft_interrupts());

    let queue_map = soft_interrupt_queues.read().await;
    assert!(!queue_map.contains_key(old_session_id));
    assert!(queue_map.contains_key(&client_session_id));
    drop(queue_map);

    let signals = shutdown_signals.read().await;
    assert!(!signals.contains_key(old_session_id));
    let registered_signal = signals
        .get(&client_session_id)
        .ok_or_else(|| anyhow!("new session should have shutdown signal"))?
        .clone();
    drop(signals);
    registered_signal.fire();
    assert!(new_cancel_signal.is_set());

    let first = client_event_rx
        .recv()
        .await
        .ok_or_else(|| anyhow!("session id event"))?;
    assert!(matches!(first, ServerEvent::SessionId { .. }));
    let second = client_event_rx
        .recv()
        .await
        .ok_or_else(|| anyhow!("done event"))?;
    assert!(matches!(second, ServerEvent::Done { id: 7 }));
    Ok(())
}
