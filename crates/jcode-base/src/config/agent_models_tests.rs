use super::*;
use std::ffi::OsString;
use std::sync::MutexGuard;

const ROLE_ENV: &[&str] = &[
    "JCODE_HOME",
    "JCODE_MODEL",
    "JCODE_SWARM_MODEL",
    "JCODE_SWARM_EFFORT",
    "JCODE_AUTOREVIEW_MODEL",
    "JCODE_AUTOREVIEW_EFFORT",
    "JCODE_AUTOJUDGE_MODEL",
    "JCODE_AUTOJUDGE_EFFORT",
    "JCODE_MEMORY_MODEL",
    "JCODE_MEMORY_EFFORT",
    "JCODE_AMBIENT_MODEL",
    "JCODE_AMBIENT_PROVIDER",
    "JCODE_AMBIENT_EFFORT",
];

struct ConfigSandbox {
    _lock: MutexGuard<'static, ()>,
    home: tempfile::TempDir,
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl ConfigSandbox {
    fn new() -> Self {
        let lock = crate::storage::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let saved = ROLE_ENV
            .iter()
            .map(|&key| (key, std::env::var_os(key)))
            .collect();
        for &key in ROLE_ENV {
            crate::env::remove_var(key);
        }
        crate::env::set_var("JCODE_HOME", home.path());
        Config::invalidate_cache();
        Self {
            _lock: lock,
            home,
            saved,
        }
    }
}

impl Drop for ConfigSandbox {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            match value {
                Some(value) => crate::env::set_var(key, value),
                None => crate::env::remove_var(key),
            }
        }
        Config::invalidate_cache();
    }
}

fn route(model: &str) -> ConfigModelRoute {
    ConfigModelRoute {
        model: model.to_string(),
        api_method: "jcode-subscription".to_string(),
        provider_label: "Jcode".to_string(),
    }
}

#[test]
fn role_models_persist_independently_and_inherit_clears_route_and_effort() {
    let sandbox = ConfigSandbox::new();
    let mut initial = Config::default();
    initial.provider.default_model = Some("main-model".to_string());
    initial.save().unwrap();
    assert!(crate::config::config().agents.swarm_route.is_none());

    for (role, model) in [
        (AgentModelRole::Swarm, "worker-model"),
        (AgentModelRole::Review, "review-model"),
        (AgentModelRole::Judge, "judge-model"),
        (AgentModelRole::Memory, "memory-model"),
        (AgentModelRole::Ambient, "ambient-model"),
    ] {
        Config::set_agent_model_selection(role, Some(&route(model)), Some(model), Some("high"))
            .unwrap();
    }

    let persisted = std::fs::read_to_string(sandbox.home.path().join("config.toml")).unwrap();
    let loaded: Config = toml::from_str(&persisted).unwrap();
    assert_eq!(loaded.provider.default_model.as_deref(), Some("main-model"));
    assert_eq!(loaded.agents.swarm_route, Some(route("worker-model")));
    assert_eq!(loaded.autoreview.route, Some(route("review-model")));
    assert_eq!(loaded.autojudge.route, Some(route("judge-model")));
    assert_eq!(loaded.agents.memory_route, Some(route("memory-model")));
    assert_eq!(loaded.ambient.route, Some(route("ambient-model")));
    assert_eq!(loaded.agents.swarm_effort.as_deref(), Some("high"));
    assert_eq!(loaded.autoreview.effort.as_deref(), Some("high"));
    assert_eq!(loaded.autojudge.effort.as_deref(), Some("high"));
    assert_eq!(loaded.agents.memory_effort.as_deref(), Some("high"));
    assert_eq!(loaded.ambient.effort.as_deref(), Some("high"));
    assert_eq!(
        crate::config::config().agents.swarm_route,
        Some(route("worker-model"))
    );

    Config::set_agent_model_selection(AgentModelRole::Review, None, None, Some("max")).unwrap();
    let inherited = crate::config::config();
    assert!(inherited.autoreview.model.is_none());
    assert!(inherited.autoreview.route.is_none());
    assert!(inherited.autoreview.effort.is_none());
    assert_eq!(inherited.autojudge.route, Some(route("judge-model")));
}

#[test]
fn role_model_update_refuses_corrupt_config_and_does_not_persist_environment() {
    let sandbox = ConfigSandbox::new();
    let path = sandbox.home.path().join("config.toml");
    let original = "[provider\ndefault_model = broken";
    std::fs::write(&path, original).unwrap();
    assert!(
        Config::set_agent_model_selection(
            AgentModelRole::Swarm,
            Some(&route("worker-model")),
            None,
            Some("high")
        )
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

    std::fs::write(&path, "[provider]\ndefault_model = \"saved-main\"\n").unwrap();
    crate::env::set_var("JCODE_MODEL", "temporary-main");
    crate::env::set_var("JCODE_AUTOJUDGE_MODEL", "temporary-judge");
    Config::set_agent_model_selection(
        AgentModelRole::Swarm,
        Some(&route("worker-model")),
        None,
        Some("high"),
    )
    .unwrap();
    let persisted: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        persisted.provider.default_model.as_deref(),
        Some("saved-main")
    );
    assert!(persisted.autojudge.model.is_none());
    assert_eq!(
        persisted.agents.swarm_model.as_deref(),
        Some("worker-model")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
            0
        );
    }
}

#[test]
fn role_model_environment_overrides_replace_routes_and_reload_effort() {
    let _sandbox = ConfigSandbox::new();
    let mut cfg = Config::default();
    cfg.agents.swarm_route = Some(route("saved-worker"));
    cfg.agents.memory_route = Some(route("saved-memory"));
    cfg.autoreview.route = Some(route("saved-review"));
    cfg.autojudge.route = Some(route("saved-judge"));
    cfg.ambient.route = Some(route("saved-ambient"));
    cfg.save().unwrap();
    for key in ROLE_ENV
        .iter()
        .filter(|key| key.ends_with("_MODEL") && **key != "JCODE_MODEL")
    {
        crate::env::set_var(key, "override-model");
    }
    for key in ROLE_ENV.iter().filter(|key| key.ends_with("_EFFORT")) {
        crate::env::set_var(key, "medium");
    }
    let active = crate::config::config();
    assert!(active.agents.swarm_route.is_none());
    assert!(active.agents.memory_route.is_none());
    assert!(active.autoreview.route.is_none());
    assert!(active.autojudge.route.is_none());
    assert!(active.ambient.route.is_none());
    assert_eq!(active.agents.swarm_model.as_deref(), Some("override-model"));
    assert_eq!(
        active.agents.memory_model.as_deref(),
        Some("override-model")
    );
    assert_eq!(active.autoreview.model.as_deref(), Some("override-model"));
    assert_eq!(active.autojudge.model.as_deref(), Some("override-model"));
    assert_eq!(active.ambient.model.as_deref(), Some("override-model"));
    assert_eq!(active.agents.swarm_effort.as_deref(), Some("medium"));
    assert_eq!(active.agents.memory_effort.as_deref(), Some("medium"));
    assert_eq!(active.autoreview.effort.as_deref(), Some("medium"));
    assert_eq!(active.autojudge.effort.as_deref(), Some("medium"));
    assert_eq!(active.ambient.effort.as_deref(), Some("medium"));
    crate::env::set_var("JCODE_AUTOJUDGE_EFFORT", "high");
    assert_eq!(
        crate::config::config().autojudge.effort.as_deref(),
        Some("high")
    );

    crate::env::remove_var("JCODE_AMBIENT_MODEL");
    crate::env::set_var("JCODE_AMBIENT_PROVIDER", "openai");
    assert!(crate::config::config().ambient.route.is_none());
    assert_eq!(
        crate::config::config().ambient.provider.as_deref(),
        Some("openai")
    );
}

#[test]
fn role_model_summary_and_change_report_show_routes_effort_and_edit_commands() {
    let _sandbox = ConfigSandbox::new();
    let mut cfg = Config::default();
    let before = toml::to_string(&cfg).unwrap();
    cfg.autojudge.route = Some(route("judge-model"));
    cfg.autojudge.effort = Some("high".to_string());
    let display = cfg.display_string();
    assert!(display.contains("/agents"));
    assert!(display.contains("/config agents"));
    assert!(display.contains("/advisor"));
    assert!(display.contains("judge-model (Jcode · jcode-subscription) · effort: high"));
    let changes = crate::config::change_report::diff_toml(&before, &toml::to_string(&cfg).unwrap());
    assert!(
        changes
            .iter()
            .any(|change| change.key == "autojudge.route.api_method")
    );
    assert!(
        changes
            .iter()
            .any(|change| change.key == "autojudge.effort")
    );
    assert!(
        changes
            .iter()
            .all(|change| change.liveness == crate::config::change_report::Liveness::Live)
    );
}

#[test]
fn role_model_defaults_accept_legacy_models_and_reject_incomplete_exact_routes() {
    let legacy: Config = toml::from_str("[agents]\nswarm_model = \"claude-opus-5\"\nswarm_effort = \"high\"\n[autojudge]\nmodel = \"gpt-5.6-astra\"\n").unwrap();
    assert_eq!(legacy.agents.swarm_model.as_deref(), Some("claude-opus-5"));
    assert!(legacy.agents.swarm_route.is_none());
    assert!(legacy.autojudge.route.is_none());
    assert!(toml::from_str::<Config>("[autojudge.route]\nmodel = \"gpt-5\"\n").is_err());
    let defaults: Config = toml::from_str(&Config::default_config_file_contents()).unwrap();
    assert!(defaults.agents.swarm_route.is_none());
    assert!(defaults.agents.memory_effort.is_none());
    assert!(defaults.autoreview.route.is_none());
    assert!(defaults.autojudge.effort.is_none());
    assert!(defaults.ambient.route.is_none());
}
