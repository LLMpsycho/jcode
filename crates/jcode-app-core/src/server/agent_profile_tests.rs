use super::*;
use crate::config::SwarmSpawnMode;
use crate::session::Session;

#[tokio::test]
async fn agent_profile_headless_and_inline_spawn_keep_name_snapshot_and_model() {
    let _sandbox = crate::auth::test_sandbox::AuthTestSandbox::new().unwrap();
    let project = tempfile::tempdir().unwrap();
    let profiles = project.path().join(".jcode/agents");
    std::fs::create_dir_all(&profiles).unwrap();
    std::fs::write(profiles.join("debug.md"), "---\nname: debug\ndescription: Diagnose bugs\nallowed-tools: read, agentgrep\n---\nPROFILE_SPAWN_SENTINEL").unwrap();
    let sessions = Arc::new(RwLock::new(HashMap::new()));
    let global = Arc::new(RwLock::new(String::new()));
    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    let members = Arc::new(RwLock::new(HashMap::new()));
    let swarms = Arc::new(RwLock::new(HashMap::new()));
    let coordinators = Arc::new(RwLock::new(HashMap::new()));
    let plans = Arc::new(RwLock::new(HashMap::new()));
    let history = Arc::new(RwLock::new(VecDeque::new()));
    let counter = Arc::new(AtomicU64::new(0));
    let (events, _) = broadcast::channel(16);
    let pool = Arc::new(crate::mcp::SharedMcpPool::new(
        crate::mcp::McpConfig::default(),
    ));
    let queues = Arc::new(RwLock::new(HashMap::new()));
    let files = crate::server::FileSnapshotLedger::new();
    let connections = Arc::new(RwLock::new(HashMap::new()));
    for (mode, profile, label, expected) in [
        (
            SwarmSpawnMode::Headless,
            Some("debug"),
            "diagnose API",
            "debug agent",
        ),
        (
            SwarmSpawnMode::Inline,
            Some("debug"),
            "inspect logs",
            "debug agent",
        ),
        (
            SwarmSpawnMode::Headless,
            None,
            "API reviewer",
            "API reviewer",
        ),
    ] {
        let id = super::super::spawn_swarm_agent(
            "coord",
            "profile-swarm",
            Some(project.path().to_string_lossy().into_owned()),
            None,
            Some(mode),
            None,
            Some(label.into()),
            profile.map(str::to_string),
            &sessions,
            &global,
            &provider,
            &members,
            &swarms,
            &coordinators,
            &plans,
            &history,
            &counter,
            &events,
            &pool,
            &queues,
            &files,
            &None,
            &connections,
        )
        .await
        .unwrap();
        let saved = Session::load(&id).unwrap();
        assert_eq!(saved.display_name(), expected);
        assert_eq!(
            saved
                .agent_profile
                .as_ref()
                .map(|profile| profile.name.as_str()),
            profile
        );
        if let Some(profile) = saved.agent_profile {
            assert_eq!(profile.content, "PROFILE_SPAWN_SENTINEL");
            assert_eq!(profile.allowed_tools.unwrap(), ["agentgrep", "read"]);
        }
        assert_eq!(
            members.read().await[&id].friendly_name.as_deref(),
            Some(expected)
        );
        assert_eq!(
            sessions.read().await[&id].lock().await.provider_model(),
            provider.model()
        );
    }
    let count = sessions.read().await.len();
    assert!(
        super::super::spawn_swarm_agent(
            "coord",
            "profile-swarm",
            Some(project.path().to_string_lossy().into_owned()),
            None,
            Some(SwarmSpawnMode::Headless),
            None,
            None,
            Some("missing".into()),
            &sessions,
            &global,
            &provider,
            &members,
            &swarms,
            &coordinators,
            &plans,
            &history,
            &counter,
            &events,
            &pool,
            &queues,
            &files,
            &None,
            &connections,
        )
        .await
        .is_err()
    );
    assert_eq!(sessions.read().await.len(), count);
}
