use super::*;

impl Registry {
    /// Obtain an investigative implementation only after the parent's current
    /// grants and implementation-owned capability have both been checked.
    /// This intentionally avoids primary-turn capture, write-ledger credit,
    /// tool hooks, and the primary conversation's compaction accounting.
    pub(crate) async fn advisor_read_tool(
        &self,
        parent_session: &str,
        name: &str,
        input: &Value,
    ) -> Result<Arc<dyn Tool>> {
        let name = Self::resolve_tool_name(name);
        anyhow::ensure!(
            matches!(name, "read" | "agentgrep"),
            "Advisor investigation only grants read and agentgrep"
        );
        // A hook can impose additional policy. An autonomous reader must not
        // bypass it, or execute a user shell command just to inspect a file.
        anyhow::ensure!(
            !crate::hooks::hook_configured("pre_tool"),
            "Advisor investigation is unavailable while a pre_tool policy hook is configured"
        );
        if let Some(policy) = session_tool_policy(parent_session) {
            let granted = |tool| {
                policy
                    .allowed_tools
                    .as_ref()
                    .is_none_or(|allowed| tool_name_is_allowed(allowed, tool))
                    && !tool_name_is_disabled(&policy.disabled_tools, tool)
            };
            anyhow::ensure!(
                granted(name) && (name != "agentgrep" || granted("read")),
                "Tool is not granted by the primary session policy"
            );
        }
        let tool = self
            .tools
            .read()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Investigative tool is unavailable"))?;
        anyhow::ensure!(
            tool.capability(input) == ToolCapability::ReadOnly,
            "Advisor investigation rejected an effectful or unknown capability"
        );
        // The session registry can contain a ReadTool backed by the primary
        // write guard's ledger. Advisor reads must never authorize main edits.
        if name == "read" {
            return Ok(Arc::new(read::ReadTool::for_advisor(1024 * 1024)));
        }
        Ok(Arc::new(AdvisorSearchTool))
    }

    #[cfg(test)]
    pub(crate) async fn advisor_test_registry() -> Self {
        let registry = Self::empty();
        {
            let mut tools = registry.tools.write().await;
            Self::insert_tool(&mut tools, "read", read::ReadTool::new());
            Self::insert_tool(&mut tools, "agentgrep", agentgrep::AgentGrepTool::new());
        }
        registry
    }
}

/// The general agentgrep engine can detach expensive searches and inherit rg
/// configuration. Autonomous investigation uses the same registered surface,
/// with a cancellable, bounded search implementation and no arbitrary commands.
struct AdvisorSearchTool;

#[async_trait::async_trait]
impl Tool for AdvisorSearchTool {
    fn name(&self) -> &str {
        "agentgrep"
    }
    fn description(&self) -> &str {
        "Bounded workspace grep or filename search"
    }
    fn parameters_schema(&self) -> Value {
        agentgrep::AgentGrepTool::new().parameters_schema()
    }
    fn capability(&self, _: &Value) -> ToolCapability {
        ToolCapability::ReadOnly
    }
    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        use tokio::io::AsyncReadExt;
        let root = ctx
            .working_dir
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Missing workspace"))?;
        let find = input.get("mode").and_then(Value::as_str) == Some("find");
        let mut command = tokio::process::Command::new("rg");
        command
            .current_dir(root)
            .env_remove("RIPGREP_CONFIG_PATH")
            .args([
                "--no-config",
                "--no-follow",
                "--no-mmap",
                "--max-depth",
                "24",
                "--max-filesize",
                "1M",
                "--threads",
                "1",
                "--color",
                "never",
            ])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(pattern) = input.get("glob").and_then(Value::as_str) {
            command.args(["--glob", pattern]);
        }
        if let Some(file_type) = input.get("type").and_then(Value::as_str) {
            command.args(["--type", file_type]);
        }
        // Last glob wins. Caller globs must never reinclude secret stores.
        for pattern in [
            "!**/.git/**",
            "!**/.ssh/**",
            "!**/.aws/**",
            "!**/.jcode/**",
            "!**/.env*",
            "!**/*credentials*",
            "!**/*secrets*",
            "!**/*.pem",
            "!**/*.key",
            "!**/*.p12",
            "!**/*.pfx",
            "!**/id_rsa*",
            "!**/id_ed25519*",
            "!**/.netrc",
            "!**/.npmrc",
            "!**/auth.json",
            "!**/oauth.json",
        ] {
            command.args(["--iglob", pattern]);
        }
        // Even an inclusive caller glob cannot opt hidden files back in.
        command.args(["--glob", "!**/.*", "--glob", "!**/.*/**"]);
        if find {
            command.arg("--files");
        } else {
            command.args([
                "--line-number",
                "--with-filename",
                "--max-count",
                "30",
                "--max-columns",
                "512",
                "--max-columns-preview",
            ]);
            if input.get("regex") != Some(&Value::Bool(true)) {
                command.arg("--fixed-strings");
            }
            command.arg("-e").arg(
                input
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
        }
        command
            .arg("--")
            .arg(input.get("path").and_then(Value::as_str).unwrap_or("."));
        let mut child = command
            .spawn()
            .map_err(|_| anyhow::anyhow!("Advisor search requires ripgrep (rg)"))?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Search output unavailable"))?
            .take(16_385)
            .read_to_end(&mut bytes)
            .await?;
        let truncated = bytes.len() > 16_384;
        if truncated {
            child.kill().await?;
            child.wait().await?;
        } else {
            anyhow::ensure!(
                matches!(child.wait().await?.code(), Some(0 | 1)),
                "Advisor search failed"
            );
        }
        let output = String::from_utf8_lossy(&bytes);
        let query = input
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let lines = output
            .lines()
            .filter(|line| !find || line.to_lowercase().contains(&query))
            .take(100)
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ToolOutput::new(format!(
            "{lines}\n{}",
            if truncated || output.lines().count() > 100 {
                "[search output truncated; narrow path or query]"
            } else {
                "[end of bounded search]"
            }
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MisclassifiedReader;

    #[async_trait]
    impl Tool for MisclassifiedReader {
        fn name(&self) -> &str {
            "read"
        }
        fn description(&self) -> &str {
            "A future tool with unknown effects"
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({})
        }
        async fn execute(&self, _: Value, _: ToolContext) -> Result<ToolOutput> {
            panic!("Unknown capability must be rejected before execution")
        }
    }

    #[tokio::test]
    async fn advisor_read_grants_do_not_trust_the_tool_name() {
        let _guard = crate::storage::lock_test_env();
        let registry = Registry::empty();
        registry
            .tools
            .write()
            .await
            .insert("read".into(), Arc::new(MisclassifiedReader));
        assert!(
            registry
                .advisor_read_tool("advisor-capability", "read", &Value::Null)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn advisor_read_grants_follow_current_parent_policy() {
        let _guard = crate::storage::lock_test_env();
        let registry = Registry::advisor_test_registry().await;
        let session = "advisor-parent-policy";
        assert!(
            registry
                .advisor_read_tool(session, "read", &Value::Null)
                .await
                .is_ok()
        );
        set_session_tool_policy(session, None, HashSet::from(["read".into()]));
        assert!(
            registry
                .advisor_read_tool(session, "read", &Value::Null)
                .await
                .is_err()
        );
        assert!(
            registry
                .advisor_read_tool(session, "agentgrep", &Value::Null)
                .await
                .is_err()
        );
        set_session_tool_policy(
            session,
            Some(HashSet::from(["read".into()])),
            HashSet::new(),
        );
        assert!(
            registry
                .advisor_read_tool(session, "agentgrep", &Value::Null)
                .await
                .is_err()
        );
        clear_session_tool_policy(session);
    }
}
