//! Mcp registration.

use super::*;

impl Registry {
    /// Register MCP tools (MCP management and server tools)
    /// Connections happen in background to avoid blocking startup.
    /// If `event_tx` is provided, sends an McpStatus event when connections complete.
    /// If `shared_pool` is provided, shared servers reuse processes from the pool.
    pub async fn register_mcp_tools(
        &self,
        event_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::protocol::ServerEvent>>,
        shared_pool: Option<std::sync::Arc<crate::mcp::SharedMcpPool>>,
        session_id: Option<String>,
    ) {
        self.register_mcp_tools_for_dir(event_tx, shared_pool, session_id, None)
            .await
    }

    /// Like [`Self::register_mcp_tools`], but resolves project-local MCP config
    /// (`.mcp.json`, `.jcode/mcp.json`, `.claude/mcp.json`) against
    /// `working_dir` instead of the server process cwd. Remote/client sessions
    /// must pass their session working directory here (issue #420).
    pub async fn register_mcp_tools_for_dir(
        &self,
        event_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::protocol::ServerEvent>>,
        shared_pool: Option<std::sync::Arc<crate::mcp::SharedMcpPool>>,
        session_id: Option<String>,
        working_dir: Option<std::path::PathBuf>,
    ) {
        use crate::mcp::McpManager;
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let mcp_manager = if let Some(pool) = shared_pool {
            let sid = session_id.unwrap_or_else(|| "unknown".to_string());
            Arc::new(RwLock::new(McpManager::with_shared_pool_for_dir(
                pool,
                sid,
                working_dir,
            )))
        } else {
            Arc::new(RwLock::new(McpManager::new()))
        };

        // Register MCP management tool immediately (with registry for dynamic tool registration)
        let mcp_tool =
            mcp::McpManagementTool::new(Arc::clone(&mcp_manager)).with_registry(self.clone());
        self.register("mcp".to_string(), Arc::new(mcp_tool) as Arc<dyn Tool>)
            .await;
        self.register(
            "mcp_search".to_string(),
            Arc::new(mcp::McpSearchTool::new(Arc::clone(&mcp_manager))) as Arc<dyn Tool>,
        )
        .await;
        self.register(
            "mcp_call".to_string(),
            Arc::new(mcp::McpCallTool::new(Arc::clone(&mcp_manager))) as Arc<dyn Tool>,
        )
        .await;

        // Check if we have enabled servers to connect to. Disabled servers stay
        // configured (visible to the mcp management tool, connectable by name)
        // but are not spawned, advertised, or shown as connecting (issue #436).
        let (enabled_count, disabled_count) = {
            let manager = mcp_manager.read().await;
            let enabled = manager
                .config()
                .servers
                .values()
                .filter(|cfg| cfg.is_enabled())
                .count();
            (enabled, manager.config().servers.len() - enabled)
        };

        if disabled_count > 0 {
            crate::logging::info(&format!(
                "MCP: {} disabled server(s) in config (kept, not spawned)",
                disabled_count
            ));
        }

        if enabled_count > 0 {
            crate::logging::info(&format!("MCP: Found {} server(s) in config", enabled_count));

            // Send immediate "connecting" status so the TUI shows loading state
            // Server names with count 0 means "connecting..."
            if let Some(ref tx) = event_tx {
                let server_names: Vec<String> = {
                    let manager = mcp_manager.read().await;
                    manager
                        .config()
                        .servers
                        .iter()
                        .filter(|(_, cfg)| cfg.is_enabled())
                        .map(|(name, _)| format!("{}:0", name))
                        .collect()
                };
                if (tx.send(crate::protocol::ServerEvent::McpStatus {
                    servers: server_names,
                }))
                .is_err()
                {
                    crate::logging::debug("Event recipient disconnected before delivery");
                }
            }

            // Advertise-early: register proxy tools for each configured server
            // from the on-disk schema cache *before* connections settle, so the
            // first locked tool snapshot already contains MCP tools and we avoid
            // the intentional prompt-cache miss entirely (#206 Phase 2). The
            // proxies connect-on-first-call. Servers with no cached schemas yet
            // (cold start, or reconfigured) fall back to the post-connect
            // registration + one-shot late-register rebuild below.
            let schema_cache = crate::mcp::McpSchemaCache::load();
            let mut advertised_servers: std::collections::BTreeSet<String> =
                std::collections::BTreeSet::new();
            {
                let config_servers: Vec<(String, crate::mcp::McpServerConfig)> = {
                    let manager = mcp_manager.read().await;
                    manager
                        .config()
                        .servers
                        .iter()
                        .filter(|(_, cfg)| cfg.is_enabled())
                        .map(|(name, cfg)| (name.clone(), cfg.clone()))
                        .collect()
                };
                let mut advertised_tool_count = 0usize;
                for (server, cfg) in &config_servers {
                    if let Some(cached) = schema_cache.tools_for(server, cfg) {
                        let tools = crate::mcp::create_mcp_tools_from_cached(
                            server,
                            cached,
                            Arc::clone(&mcp_manager),
                        );
                        advertised_tool_count += tools.len();
                        for (name, tool) in tools {
                            self.register(name, tool).await;
                        }
                        advertised_servers.insert(server.clone());
                    }
                }
                if advertised_tool_count > 0 {
                    crate::logging::info(&format!(
                        "MCP: advertised {} cached tool(s) from {} server(s) at spawn \
                         (connect-on-first-call); zero prompt-cache miss expected (#206)",
                        advertised_tool_count,
                        advertised_servers.len()
                    ));
                    // Reflect the advertised tools in the status indicator
                    // immediately so the UI shows them before connections settle.
                    if let Some(ref tx) = event_tx {
                        let mut counts: std::collections::BTreeMap<String, usize> =
                            std::collections::BTreeMap::new();
                        for (server, cfg) in &config_servers {
                            if let Some(cached) = schema_cache.tools_for(server, cfg) {
                                counts.insert(server.clone(), cached.len());
                            }
                        }
                        let servers: Vec<String> = counts
                            .into_iter()
                            .map(|(name, count)| format!("{}:{}", name, count))
                            .collect();
                        if (tx.send(crate::protocol::ServerEvent::McpStatus { servers })).is_err() {
                            crate::logging::debug("Event recipient disconnected before delivery");
                        }
                    }
                }
            }

            // Spawn connection and tool registration in background
            let registry = self.clone();
            tokio::spawn(async move {
                let (successes, failures) = {
                    let manager = mcp_manager.write().await;
                    manager.connect_all().await.unwrap_or((0, Vec::new()))
                };

                if successes > 0 {
                    crate::logging::info(&format!("MCP: Connected to {} server(s)", successes));
                }
                if !failures.is_empty() {
                    for (name, error) in &failures {
                        crate::logging::event_rate_limited(
                            crate::logging::LogLevel::Error,
                            &format!("mcp_register_failed:{name}"),
                            std::time::Duration::from_secs(60),
                            "MCP_REGISTER_FAILED",
                            vec![("server", name.to_string()), ("error", error.to_string())],
                        );
                    }
                }

                // Register MCP server tools and collect server info
                let tools = crate::mcp::create_mcp_tools(Arc::clone(&mcp_manager)).await;
                let mut server_counts: std::collections::BTreeMap<String, usize> =
                    std::collections::BTreeMap::new();
                for (name, tool) in &tools {
                    if let Some(rest) = name.strip_prefix("mcp__")
                        && let Some((server, _)) = rest.split_once("__")
                    {
                        *server_counts.entry(server.to_string()).or_default() += 1;
                    }
                    // Idempotent: advertise-early may have already registered an
                    // identical proxy. Re-registering refreshes it with the live
                    // schema, which is correct (handles schema drift).
                    registry.register(name.clone(), tool.clone()).await;
                }

                // Reconcile the on-disk schema cache with the live schemas so the
                // next spawn can advertise the up-to-date tools with zero cache
                // miss. Group live tool defs by server and update each entry
                // under the current config fingerprint; prune servers that are
                // no longer configured. (#206 Phase 2)
                {
                    // Live tool defs grouped by server, plus a snapshot of the
                    // configured servers, captured under one read lock.
                    type LiveToolsByServer =
                        std::collections::BTreeMap<String, Vec<crate::mcp::McpToolDef>>;
                    type ConfigSnapshot = Vec<(String, crate::mcp::McpServerConfig)>;
                    let (live_by_server, config_snapshot): (LiveToolsByServer, ConfigSnapshot) = {
                        let manager = mcp_manager.read().await;
                        let mut grouped: std::collections::BTreeMap<
                            String,
                            Vec<crate::mcp::McpToolDef>,
                        > = std::collections::BTreeMap::new();
                        for (server, def) in manager.all_tools().await {
                            grouped.entry(server).or_default().push(def);
                        }
                        let configs = manager
                            .config()
                            .servers
                            .iter()
                            .map(|(name, cfg)| (name.clone(), cfg.clone()))
                            .collect();
                        (grouped, configs)
                    };

                    let mut cache = crate::mcp::McpSchemaCache::load();
                    let mut dirty = false;
                    for (server, cfg) in &config_snapshot {
                        if let Some(defs) = live_by_server.get(server) {
                            // Only cache servers that actually exposed tools.
                            if cache.update(server, cfg, defs.clone()) {
                                dirty = true;
                            }
                        }
                    }
                    let configured_names: Vec<String> =
                        config_snapshot.iter().map(|(n, _)| n.clone()).collect();
                    if cache.retain_servers(&configured_names) {
                        dirty = true;
                    }
                    if dirty {
                        cache.save();
                        crate::logging::info(
                            "MCP: updated on-disk tool-schema cache from live connection (#206)",
                        );
                    }
                }

                // Notify client of MCP status
                if let Some(tx) = event_tx {
                    let servers: Vec<String> = server_counts
                        .into_iter()
                        .map(|(name, count)| format!("{}:{}", name, count))
                        .collect();
                    if (tx.send(crate::protocol::ServerEvent::McpStatus { servers })).is_err() {
                        crate::logging::debug("Event recipient disconnected before delivery");
                    }
                }
            });
        }
    }
}
