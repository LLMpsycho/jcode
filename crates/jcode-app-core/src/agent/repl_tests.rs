use super::*;

struct ReplCommandProvider;

#[async_trait::async_trait]
impl crate::provider::Provider for ReplCommandProvider {
    async fn complete(
        &self,
        _messages: &[crate::message::Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        panic!("REPL commands and bare skill activation must not call a provider");
    }

    fn name(&self) -> &str {
        "repl-command-test"
    }

    fn fork(&self) -> Arc<dyn crate::provider::Provider> {
        Arc::new(Self)
    }
}

async fn repl_agent(working_dir: &std::path::Path) -> Agent {
    let provider: Arc<dyn crate::provider::Provider> = Arc::new(ReplCommandProvider);
    let registry = crate::tool::Registry::new(provider.clone()).await;
    *registry.skills().write().await = crate::skill::SkillRegistry::default();
    let mut agent = Agent::new(provider, registry);
    agent.session.working_dir = Some(working_dir.to_string_lossy().into_owned());
    agent.memory_enabled = false;
    agent
}

async fn dispatch_repl(agent: &mut Agent, lines: &[&str]) -> Vec<String> {
    let mut lines = lines.iter();
    let mut output = Vec::new();
    agent
        .repl_with_input(
            || Ok(lines.next().map(|line| (*line).to_string())),
            |line| output.push(line.to_string()),
        )
        .await
        .unwrap();
    output
}

#[tokio::test]
async fn advisor_repl_commands_show_tui_guidance_without_skill_or_provider_dispatch() {
    let sandbox = crate::auth::test_sandbox::AuthTestSandbox::new().unwrap();
    let mut agent = repl_agent(sandbox.root()).await;
    let initial_messages = serde_json::to_value(&agent.session.messages).unwrap();
    let output = dispatch_repl(
        &mut agent,
        &[
            "/advisor",
            "  /advisor status  ",
            "/advisor models",
            "/advisor off",
            "/advisor\tack note-1",
            "quit",
            "/this-must-not-run",
        ],
    )
    .await;

    let guidance: Vec<_> = output
        .iter()
        .filter(|line| line.contains("Advisor model selection"))
        .collect();
    assert_eq!(guidance.len(), 5);
    assert!(guidance.iter().all(|line| line.contains("run `jcode`")));
    assert!(
        guidance
            .iter()
            .all(|line| line.contains("reasoning effort"))
    );
    assert!(!output.iter().any(|line| line.contains("Unknown skill")));
    assert!(agent.active_skill.is_none());
    assert_eq!(
        serde_json::to_value(&agent.session.messages).unwrap(),
        initial_messages
    );
}

#[tokio::test]
async fn advisor_repl_dispatch_preserves_other_skills_and_unknown_names() {
    let sandbox = crate::auth::test_sandbox::AuthTestSandbox::new().unwrap();
    for (directory, name) in [("advisory", "advisory"), ("custom", "My Custom Skill")] {
        let skill_dir = sandbox.root().join(".jcode/skills").join(directory);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: A regression-test skill\n---\nTest skill.\n"),
        )
        .unwrap();
    }
    let mut agent = repl_agent(sandbox.root()).await;
    let initial_messages = serde_json::to_value(&agent.session.messages).unwrap();
    let output = dispatch_repl(
        &mut agent,
        &["/advisory", "/My Custom Skill", "/advisor-missing"],
    )
    .await;

    assert!(
        output
            .iter()
            .any(|line| line == "Activating skill: advisory")
    );
    assert!(
        output
            .iter()
            .any(|line| line == "Activating skill: My Custom Skill")
    );
    assert!(
        output
            .iter()
            .any(|line| line == "Unknown skill: /advisor-missing")
    );
    assert!(
        !output
            .iter()
            .any(|line| line.contains("Advisor model selection"))
    );
    assert_eq!(agent.active_skill.as_deref(), Some("My Custom Skill"));
    assert_eq!(
        serde_json::to_value(&agent.session.messages).unwrap(),
        initial_messages
    );
}
