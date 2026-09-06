use super::*;
use crate::config::{AgentModelRole, Config, ConfigModelRoute};
use crate::provider::ModelRoute;
use crate::tui::{AgentModelTarget, PickerAction};

fn role_route(model: &str, method: &str, provider: &str) -> ModelRoute {
    ModelRoute {
        model: model.into(),
        api_method: method.into(),
        provider: provider.into(),
        available: true,
        detail: "authenticated catalog fixture".into(),
        cheapness: None,
    }
}

fn role_catalog_app() -> App {
    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_provider_name = Some("OpenAI".into());
    app.remote_provider_model = Some("main-model".into());
    app.remote_reasoning_effort = Some("low".into());
    app.remote_model_options = vec![
        role_route("gpt-5.5", "jcode-subscription", "Jcode"),
        role_route("openai/gpt-5.5", "openrouter", "Azure"),
        role_route("gpt-5.5", "openai-compatible:team-work", "Team Work"),
        role_route("gemini-2.5-pro", "code-assist-oauth", "Gemini"),
    ];
    app.remote_available_entries = app
        .remote_model_options
        .iter()
        .map(|route| route.model.clone())
        .collect();
    app
}

fn saved_role_fields(
    target: AgentModelTarget,
) -> (Option<String>, Option<ConfigModelRoute>, Option<String>) {
    let cfg = Config::load();
    match target {
        AgentModelTarget::Swarm => (
            cfg.agents.swarm_model,
            cfg.agents.swarm_route,
            cfg.agents.swarm_effort,
        ),
        AgentModelTarget::Review => (
            cfg.autoreview.model,
            cfg.autoreview.route,
            cfg.autoreview.effort,
        ),
        AgentModelTarget::Judge => (
            cfg.autojudge.model,
            cfg.autojudge.route,
            cfg.autojudge.effort,
        ),
        AgentModelTarget::Memory => (
            cfg.agents.memory_model,
            cfg.agents.memory_route,
            cfg.agents.memory_effort,
        ),
        AgentModelTarget::Ambient => (cfg.ambient.model, cfg.ambient.route, cfg.ambient.effort),
        _ => panic!("expected a persistent role"),
    }
}

fn choose_role_entry(app: &mut App, predicate: impl Fn(&crate::tui::PickerEntry) -> bool) {
    let picker = app.inline_interactive_state.as_mut().expect("role picker");
    let index = picker
        .entries
        .iter()
        .position(predicate)
        .expect("requested role option");
    picker.selected = picker
        .filtered
        .iter()
        .position(|candidate| *candidate == index)
        .unwrap();
    picker.column = picker.max_navigable_column();
    app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
}

fn assert_role_target_and_filter(app: &App, target: AgentModelTarget, filter: &str) {
    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("role picker remains open");
    assert_eq!(picker.filter, filter);
    assert!(
        picker.entries.iter().all(|entry| matches!(
            entry.action,
            PickerAction::AgentModelChoice { target: actual, .. } if actual == target
        )),
        "catalog refresh must never turn a role picker into the primary picker"
    );
    assert!(!picker.filtered.is_empty());
}

fn stage_role_catalog_result(
    app: &mut App,
) -> std::sync::mpsc::Sender<anyhow::Result<super::super::ModelPickerRoutesResult>> {
    let signature = app
        .model_picker_cache
        .as_ref()
        .expect("raw catalog cache")
        .signature
        .clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    app.model_picker_load_request_id = app.model_picker_load_request_id.wrapping_add(1);
    app.pending_model_picker_load = Some(super::super::PendingModelPickerLoad {
        request_id: app.model_picker_load_request_id,
        signature,
        picker_started: Instant::now(),
        receiver,
    });
    sender
}

#[test]
fn agent_role_picker_lists_all_seven_roles() {
    with_temp_jcode_home(|| {
        let mut app = role_catalog_app();
        app.open_agents_picker();
        let picker = app.inline_interactive_state.as_ref().unwrap();
        let targets: Vec<_> = picker
            .entries
            .iter()
            .filter_map(|entry| match entry.action {
                PickerAction::AgentTarget(target) => Some(target),
                _ => None,
            })
            .collect();
        assert_eq!(
            targets,
            vec![
                AgentModelTarget::Main,
                AgentModelTarget::Swarm,
                AgentModelTarget::Advisor,
                AgentModelTarget::Review,
                AgentModelTarget::Judge,
                AgentModelTarget::Memory,
                AgentModelTarget::Ambient,
            ]
        );
        assert_eq!(app.remote_provider_model.as_deref(), Some("main-model"));
    });
}

#[test]
fn agent_role_picker_discovers_named_profiles_and_shows_details_without_changing_model() {
    with_temp_jcode_home(|| {
        let project = tempfile::tempdir().unwrap();
        let global_profiles = crate::storage::jcode_dir().unwrap().join("agents");
        let project_profiles = project.path().join(".jcode/agents");
        for (directory, name, description) in [
            (
                &global_profiles,
                "devops",
                "Review deployment infrastructure",
            ),
            (
                &project_profiles,
                "debug",
                "Diagnose a reproducible failure",
            ),
        ] {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join(format!("{name}.md")), format!(
                "---\nname: {name}\ndescription: {description}\nallowed-tools: [read, agentgrep]\neffort: high\n---\nPrivate {name} role instructions\n"
            )).unwrap();
        }
        let mut app = role_catalog_app();
        app.session.working_dir = Some(project.path().to_string_lossy().into_owned());
        app.input = "/config".into();
        app.submit_input();
        let config_text = &app.display_messages.last().unwrap().content;
        assert!(config_text.contains("devops agent") && config_text.contains("debug agent"));
        assert!(!config_text.contains("Private devops role instructions"));

        let prompt = crate::prompt::load_swarm_prompt(Some(project.path()));
        assert!(prompt.contains("`devops`") && prompt.contains("`debug`"));
        assert!(prompt.contains("Review deployment infrastructure"));
        assert!(!prompt.contains("Private devops role instructions"));

        app.input = "/config agents".into();
        app.submit_input();
        let picker = app.inline_interactive_state.as_ref().unwrap();
        assert!(
            picker
                .entries
                .iter()
                .any(|entry| entry.name == "devops agent")
        );
        assert!(
            picker
                .entries
                .iter()
                .any(|entry| entry.name == "debug agent")
        );
        choose_role_entry(
            &mut app,
            |entry| matches!(&entry.action, PickerAction::AgentProfile(name) if name == "devops"),
        );
        assert!(app.inline_interactive_state.is_none());
        assert!(
            app.display_messages
                .last()
                .unwrap()
                .content
                .contains("Private devops role instructions")
        );

        for command in [
            "/agents debug",
            "/config agents debug",
            "/config models debug",
        ] {
            assert!(
                app.get_suggestions_for(command)
                    .iter()
                    .any(|(suggestion, _)| suggestion == command)
            );
            app.input = command.into();
            app.submit_input();
            assert!(
                app.display_messages
                    .last()
                    .unwrap()
                    .content
                    .contains("Private debug role instructions")
            );
        }
        assert_eq!(app.remote_provider_model.as_deref(), Some("main-model"));
        app.input = "/agents missing-profile".into();
        app.submit_input();
        assert!(
            app.display_messages
                .last()
                .unwrap()
                .content
                .contains("Unknown agent profile")
        );
    });
}

#[test]
fn agent_role_picker_enter_persists_exact_connection_and_effort_without_switching_main() {
    with_temp_jcode_home(|| {
        let mut initial = Config::default();
        initial.provider.default_model = Some("saved-main".into());
        initial.save().unwrap();
        Config::invalidate_cache();
        for (target, model, method, provider, legacy) in [
            (
                AgentModelTarget::Swarm,
                "gpt-5.5",
                "jcode-subscription",
                "Jcode",
                "gpt-5.5",
            ),
            (
                AgentModelTarget::Review,
                "openai/gpt-5.5",
                "openrouter",
                "Azure",
                "openai/gpt-5.5@Azure",
            ),
            (
                AgentModelTarget::Judge,
                "gpt-5.5",
                "openai-compatible:team-work",
                "Team Work",
                "team-work:gpt-5.5",
            ),
        ] {
            let mut app = role_catalog_app();
            app.open_agent_model_picker(target);
            choose_role_entry(&mut app, |entry| {
                entry.effort.as_deref() == Some("high")
                    && entry
                        .options
                        .iter()
                        .any(|option| option.api_method == method && option.provider == provider)
            });
            let (saved_model, saved_route, saved_effort) = saved_role_fields(target);
            assert_eq!(saved_model.as_deref(), Some(legacy));
            assert_eq!(
                saved_route,
                Some(ConfigModelRoute {
                    model: model.into(),
                    api_method: method.into(),
                    provider_label: provider.into(),
                })
            );
            assert_eq!(saved_effort.as_deref(), Some("high"));
            assert_eq!(app.remote_provider_model.as_deref(), Some("main-model"));
            assert_eq!(app.remote_reasoning_effort.as_deref(), Some("low"));
            assert!(app.pending_route_selection.is_none());
            assert!(app.pending_model_switch.is_none());
            assert!(app.pending_reasoning_effort.is_none());
            assert_eq!(
                Config::load().provider.default_model.as_deref(),
                Some("saved-main")
            );
        }
    });
}

#[test]
fn agent_role_picker_memory_allows_catalog_models_outside_claude_and_openai() {
    with_temp_jcode_home(|| {
        let mut app = role_catalog_app();
        app.open_agent_model_picker(AgentModelTarget::Memory);
        choose_role_entry(&mut app, |entry| {
            entry.name == "gemini-2.5-pro"
                && entry
                    .options
                    .iter()
                    .any(|option| option.api_method == "code-assist-oauth" && option.available)
        });
        let (_, route, effort) = saved_role_fields(AgentModelTarget::Memory);
        assert_eq!(
            route,
            Some(ConfigModelRoute {
                model: "gemini-2.5-pro".into(),
                api_method: "code-assist-oauth".into(),
                provider_label: "Gemini".into(),
            })
        );
        assert!(effort.is_none());
        assert_eq!(app.remote_provider_model.as_deref(), Some("main-model"));
    });
}

#[test]
fn agent_role_picker_inherit_clears_model_route_and_effort_for_each_saved_role() {
    with_temp_jcode_home(|| {
        for (target, role) in [
            (AgentModelTarget::Swarm, AgentModelRole::Swarm),
            (AgentModelTarget::Review, AgentModelRole::Review),
            (AgentModelTarget::Judge, AgentModelRole::Judge),
            (AgentModelTarget::Memory, AgentModelRole::Memory),
            (AgentModelTarget::Ambient, AgentModelRole::Ambient),
        ] {
            Config::set_agent_model_selection(
                role,
                Some(&ConfigModelRoute {
                    model: "gpt-5.5".into(),
                    api_method: "jcode-subscription".into(),
                    provider_label: "Jcode".into(),
                }),
                Some("gpt-5.5"),
                Some("high"),
            )
            .unwrap();
            let mut app = role_catalog_app();
            app.open_agent_model_picker(target);
            choose_role_entry(&mut app, |entry| {
                matches!(
                    entry.action,
                    PickerAction::AgentModelChoice {
                        clear_override: true,
                        ..
                    }
                )
            });
            assert_eq!(saved_role_fields(target), (None, None, None));
        }
    });
}

#[test]
fn agent_role_picker_unavailable_saved_routes_and_loading_placeholders_cannot_be_saved() {
    with_temp_jcode_home(|| {
        let saved = ConfigModelRoute {
            model: "missing-worker".into(),
            api_method: "jcode-subscription".into(),
            provider_label: "Jcode".into(),
        };
        Config::set_agent_model_selection(AgentModelRole::Swarm, Some(&saved), None, Some("high"))
            .unwrap();
        let mut app = role_catalog_app();
        app.open_agent_model_picker(AgentModelTarget::Swarm);
        let missing = app
            .inline_interactive_state
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.name.starts_with("missing-worker"))
            .unwrap();
        assert!(missing.is_current);
        assert!(missing.options.iter().all(|option| !option.available));
        choose_role_entry(&mut app, |entry| entry.name.starts_with("missing-worker"));
        assert_eq!(saved_role_fields(AgentModelTarget::Swarm).1, Some(saved));

        for method in ["current", "remote-catalog"] {
            let mut app = role_catalog_app();
            app.remote_model_options = vec![role_route("private-placeholder", method, "Loading")];
            app.remote_available_entries.clear();
            app.open_agent_model_picker(AgentModelTarget::Ambient);
            let picker = app.inline_interactive_state.as_ref().unwrap();
            let placeholder = picker
                .entries
                .iter()
                .find(|entry| {
                    entry
                        .options
                        .iter()
                        .any(|option| option.api_method == method)
                })
                .unwrap();
            assert!(placeholder.options.iter().all(|option| !option.available));
            choose_role_entry(&mut app, |entry| {
                entry
                    .options
                    .iter()
                    .any(|option| option.api_method == method)
            });
            assert_eq!(
                saved_role_fields(AgentModelTarget::Ambient),
                (None, None, None)
            );
        }
    });
}

#[test]
fn agent_role_picker_cache_refresh_and_async_completion_preserve_role_and_filter() {
    with_temp_jcode_home(|| {
        let mut app = role_catalog_app();
        app.open_agent_model_picker(AgentModelTarget::Judge);
        app.inline_interactive_state.as_mut().unwrap().filter = "gpt".into();
        App::apply_inline_interactive_filter(app.inline_interactive_state.as_mut().unwrap());
        // First rebuild hits the raw /model cache and must reapply the role.
        app.refresh_open_model_picker_after_catalog_update();
        assert_role_target_and_filter(&app, AgentModelTarget::Judge, "gpt");
        // A new catalog must preserve the role through a synchronous rebuild.
        app.remote_model_options
            .push(role_route("gpt-5.6", "openai-oauth", "OpenAI"));
        app.invalidate_model_picker_cache();
        app.refresh_open_model_picker_after_catalog_update();
        assert_role_target_and_filter(&app, AgentModelTarget::Judge, "gpt");
        // Finally deliver the same kind of channel result used by the worker.
        let sender = stage_role_catalog_result(&mut app);
        assert!(
            sender
                .send(Ok(super::super::ModelPickerRoutesResult {
                    routes: app.remote_model_options.clone(),
                    routes_ms: 0,
                }))
                .is_ok()
        );
        assert!(app.poll_model_picker_load());
        assert_role_target_and_filter(&app, AgentModelTarget::Judge, "gpt");
        choose_role_entry(&mut app, |entry| {
            entry.effort.as_deref() == Some("high")
                && entry
                    .options
                    .iter()
                    .any(|option| option.api_method == "jcode-subscription")
        });
        assert!(saved_role_fields(AgentModelTarget::Judge).1.is_some());
        assert!(app.pending_model_switch.is_none());
    });
}

#[test]
fn agent_role_picker_late_async_completion_does_not_reopen_after_escape() {
    with_temp_jcode_home(|| {
        let mut app = role_catalog_app();
        app.open_agent_model_picker(AgentModelTarget::Review);
        let sender = stage_role_catalog_result(&mut app);
        app.handle_inline_interactive_key(KeyCode::Esc, KeyModifiers::NONE)
            .unwrap();
        assert!(app.inline_interactive_state.is_none());
        let _ = sender.send(Ok(super::super::ModelPickerRoutesResult {
            routes: app.remote_model_options.clone(),
            routes_ms: 0,
        }));
        assert!(!app.poll_model_picker_load());
        assert!(app.inline_interactive_state.is_none());
        assert_eq!(
            saved_role_fields(AgentModelTarget::Review),
            (None, None, None)
        );
    });
}
