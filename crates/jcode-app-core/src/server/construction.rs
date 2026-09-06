use super::*;

impl Server {
    pub fn new(provider: Arc<dyn Provider>) -> Self {
        Self::new_with_name(provider, None)
    }

    pub fn new_with_name(provider: Arc<dyn Provider>, server_name: Option<String>) -> Self {
        use crate::id::{new_id, new_memorable_server_id, server_icon};

        // Register the live provider so background helpers (the memory sidecar)
        // can make cheap model calls on whatever provider the user is running.
        // Without this, the sidecar only works on OpenAI/Claude OAuth and
        // silently degrades (rerank -> hybrid order, no relevance/extraction) on
        // Copilot, Antigravity, Gemini, Cursor, Bedrock, and OpenRouter.
        crate::provider::set_active_provider(Arc::clone(&provider));

        let (event_tx, _) = broadcast::channel(1024);
        let (client_debug_response_tx, _) = broadcast::channel(64);

        // Generate a memorable server name unless the operator configured a
        // stable one for long-lived remote runtimes.
        let (id, name) = match configured_server_name(server_name) {
            Some(name) => (new_id(&format!("server_{name}")), name),
            None => new_memorable_server_id(),
        };
        let icon = server_icon(&name).to_string();
        let identity = ServerIdentity {
            id,
            name,
            icon,
            git_hash: jcode_build_meta::git_hash().to_string(),
            version: jcode_build_meta::version().to_string(),
        };
        crate::process_title::set_server_title(&identity.name);

        let file_snapshots = FileSnapshotLedger::new();
        let lsp_pool = Arc::new(jcode_lsp::LspServicePool::new());
        let config = crate::config::config();
        let dap_service = if config.dap.enabled {
            match crate::tool::dap::DapService::from_config(&config.dap) {
                Ok(service) => Some(service),
                Err(error) => {
                    crate::logging::error(&format!(
                        "DAP is disabled because its configuration is invalid: {error}"
                    ));
                    None
                }
            }
        } else {
            None
        };

        // Initialize the background runner even when ambient mode is disabled so
        // session-targeted scheduled tasks still have a live delivery loop.
        let ambient_runner = {
            let safety = Arc::new(crate::safety::SafetySystem::new());
            let handle =
                AmbientRunnerHandle::new_with_file_snapshots(safety, file_snapshots.clone());
            crate::tool::ambient::init_schedule_runner(handle.clone());
            Some(handle)
        };

        let LoadedSwarmRuntimeState {
            plans: restored_swarm_plans,
            coordinators: restored_swarm_coordinators,
            members: restored_swarm_members,
            swarms_by_id: restored_swarms_by_id,
        } = load_persisted_swarm_runtime_state();

        Self {
            provider,
            socket_path: socket_path(),
            debug_socket_path: debug_socket_path(),
            gateway_config_override: None,
            identity,
            event_tx,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            is_processing: Arc::new(RwLock::new(false)),
            session_id: Arc::new(RwLock::new(String::new())),
            client_count: Arc::new(RwLock::new(0)),
            client_connections: Arc::new(RwLock::new(HashMap::new())),
            file_touch: FileTouchService::new(),
            file_snapshots,
            lsp_pool,
            dap_service,
            swarm_state: SwarmState::new(
                restored_swarm_members,
                restored_swarms_by_id,
                restored_swarm_plans,
                restored_swarm_coordinators,
            ),
            shared_context: Arc::new(RwLock::new(HashMap::new())),
            client_debug_state: Arc::new(RwLock::new(ClientDebugState::default())),
            client_debug_response_tx,
            debug_jobs: Arc::new(RwLock::new(HashMap::new())),
            channel_subscriptions: Arc::new(RwLock::new(HashMap::new())),
            channel_subscriptions_by_session: Arc::new(RwLock::new(HashMap::new())),
            event_history: Arc::new(RwLock::new(std::collections::VecDeque::new())),
            event_counter: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            swarm_event_tx: broadcast::channel(256).0,
            ambient_runner,
            mcp_pool: Arc::new(OnceCell::new()),
            shutdown_signals: Arc::new(RwLock::new(HashMap::new())),
            soft_interrupt_queues: Arc::new(RwLock::new(HashMap::new())),
            await_members_runtime: AwaitMembersRuntime::default(),
            swarm_mutation_runtime: SwarmMutationRuntime::default(),
        }
    }

    pub fn new_with_paths(
        provider: Arc<dyn Provider>,
        socket_path: PathBuf,
        debug_socket_path: PathBuf,
    ) -> Self {
        let mut server = Self::new(provider);
        server.socket_path = socket_path;
        server.debug_socket_path = debug_socket_path;
        server
    }

    pub fn with_gateway_config(mut self, gateway_config: crate::gateway::GatewayConfig) -> Self {
        self.gateway_config_override = Some(gateway_config);
        self
    }
}
