//! Named advisors share primary evidence but retain independent controls and context.
use super::{AdvisorManager, AdvisorStatus, AdvisorTurnInput, AdvisorUpdateContext};
use crate::config::{AdvisorConfig, AdvisorRosterEntry};
use crate::provider::Provider;
use anyhow::{Result, bail, ensure};
use jcode_agent_runtime::SoftInterruptQueue;
use std::collections::HashSet;
use std::sync::Arc;

mod project;
pub use project::config_for_owner;

pub const DEFAULT_ADVISOR: &str = "default";
const MAX_ADVISORS: usize = 8;
const MAX_INSTRUCTIONS_BYTES: usize = 16 * 1024;

pub struct ResolvedAdvisorEntry {
    pub name: String,
    pub config: AdvisorConfig,
    pub instructions: String,
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

/// Preserve the legacy default runtime key and namespace named advisor keys.
pub fn runtime_session_key(owner: &str, name: &str) -> String {
    if name == DEFAULT_ADVISOR {
        owner.to_string()
    } else {
        format!("advisor-roster:{}:{owner}:{name}", owner.len())
    }
}

pub fn entries(config: &AdvisorConfig) -> Result<Vec<ResolvedAdvisorEntry>> {
    ensure!(
        config.roster.len() <= MAX_ADVISORS,
        "advisor roster exceeds 8 entries"
    );
    let default = AdvisorRosterEntry::default();
    let roster = if config.roster.is_empty() {
        std::slice::from_ref(&default)
    } else {
        &config.roster
    };
    let mut names = HashSet::new();
    let mut resolved = Vec::with_capacity(roster.len());
    for entry in roster {
        ensure!(
            valid_name(&entry.name),
            "advisor name must be 1–48 lowercase letters, digits, '-' or '_'"
        );
        ensure!(
            names.insert(entry.name.clone()),
            "duplicate advisor name: {}",
            entry.name
        );
        let mut instructions = config.instructions.clone().unwrap_or_else(String::new);
        if let Some(specialization) = &entry.instructions {
            if !instructions.is_empty() {
                instructions.push_str("\n\n");
            }
            instructions.push_str(specialization);
        }
        ensure!(
            instructions.len() <= MAX_INSTRUCTIONS_BYTES,
            "advisor instructions exceed 16 KiB"
        );
        let mut merged = config.clone();
        merged.roster.clear();
        merged.enabled &= entry.enabled;
        merged.instructions = Some(instructions.clone());
        if entry.model.is_some() || entry.route.is_some() {
            merged.model = entry.model.clone();
            merged.route = entry.route.clone();
        }
        merged.effort = entry.effort.clone().or(merged.effort);
        resolved.push(ResolvedAdvisorEntry {
            name: entry.name.clone(),
            config: merged,
            instructions,
        });
    }
    Ok(resolved)
}

pub fn entry(config: &AdvisorConfig, name: &str) -> Result<ResolvedAdvisorEntry> {
    if let Some(entry) = entries(config)?
        .into_iter()
        .find(|entry| entry.name == name)
    {
        return Ok(entry);
    }
    bail!("advisor '{name}' is not configured; inspect /advisor status for available names")
}

/// Include inactive or recently removed entries so /advisor off still works
/// after the project configuration becomes invalid or changes its roster.
pub fn known_runtime_keys(manager: &AdvisorManager, owner: &str) -> Result<Vec<String>> {
    let prefix = format!("advisor-roster:{}:{owner}:", owner.len());
    let sessions = manager.sessions.lock().map_err(|_| {
        anyhow::anyhow!("advisor state unavailable; disable could not be confirmed")
    })?;
    Ok(sessions
        .iter()
        .filter(|(key, runtime)| {
            key.as_str() == owner
                || runtime.owner_session_id == owner
                || key.strip_prefix(&prefix).is_some_and(valid_name)
        })
        .map(|(key, _)| key.clone())
        .collect())
}

fn owner_control_key(owner: &str) -> String {
    format!("advisor-roster-control:{}:{owner}", owner.len())
}

pub fn owner_enabled(manager: &AdvisorManager, owner: &str) -> bool {
    let key = owner_control_key(owner);
    manager.resume(&key);
    manager.is_enabled(&key, true)
}

pub fn enable_owner(manager: &AdvisorManager, owner: &str) -> Result<()> {
    manager.set_enabled(&owner_control_key(owner), true)
}

pub fn disable_all(manager: &AdvisorManager, owner: &str, config: &AdvisorConfig) -> Result<()> {
    let mut keys = known_runtime_keys(manager, owner)?;
    keys.push(owner.into());
    keys.push(owner_control_key(owner));
    if let Ok(entries) = entries(config) {
        keys.extend(
            entries
                .iter()
                .map(|entry| runtime_session_key(owner, &entry.name)),
        );
    }
    keys.sort();
    keys.dedup();
    let mut failed = false;
    for key in keys {
        manager.resume(&key);
        failed |= manager.set_enabled(&key, false).is_err();
    }
    ensure!(
        !failed,
        "advisors disabled in memory but some controls could not be saved; retry /advisor off"
    );
    Ok(())
}

pub fn is_enabled(
    manager: &AdvisorManager,
    owner: &str,
    global: &AdvisorConfig,
    working_dir: Option<&std::path::Path>,
) -> bool {
    let resolved = config_for_owner(owner, global, working_dir).and_then(|config| entries(&config));
    let entries = match resolved {
        Ok(entries) => entries,
        Err(error) => {
            configuration_failed(manager, owner, &error.to_string());
            return false;
        }
    };
    let owner_enabled = owner_enabled(manager, owner);
    entries.into_iter().any(|entry| {
        let key = runtime_session_key(owner, &entry.name);
        manager.resume(&key);
        manager.is_enabled(&key, owner_enabled && entry.config.enabled)
    })
}

pub fn schedule_updates(
    manager: &Arc<AdvisorManager>,
    owner_session_id: String,
    provider: Arc<dyn Provider>,
    queue: SoftInterruptQueue,
    input: AdvisorTurnInput,
    config: AdvisorConfig,
    context: AdvisorUpdateContext,
) -> bool {
    let resolved = config_for_owner(&owner_session_id, &config, context.working_dir.as_deref())
        .and_then(|config| entries(&config));
    let entries = match resolved {
        Ok(entries) => entries,
        Err(error) => {
            configuration_failed(manager, &owner_session_id, &error.to_string());
            return false;
        }
    };
    let active_keys = entries
        .iter()
        .map(|entry| runtime_session_key(&owner_session_id, &entry.name))
        .collect::<Vec<_>>();
    manager.retain_advisors(&owner_session_id, &active_keys);
    let owner_enabled = owner_enabled(manager, &owner_session_id);
    let mut started = false;
    for mut entry in entries {
        entry.config.enabled &= owner_enabled;
        let key = runtime_session_key(&owner_session_id, &entry.name);
        manager.resume(&key);
        let mut advisor_context = context.clone();
        advisor_context.owner_session_id = owner_session_id.clone();
        advisor_context.advisor_label = entry.name;
        if !advisor_context.instructions.is_empty() && !entry.instructions.is_empty() {
            advisor_context.instructions.push_str("\n\n");
        }
        advisor_context.instructions.push_str(&entry.instructions);
        started |= manager.schedule_update(
            key,
            provider.fork(),
            Arc::clone(&queue),
            input.clone(),
            entry.config,
            advisor_context,
        );
    }
    started
}

fn configuration_failed(manager: &AdvisorManager, session: &str, error: &str) {
    manager.cancel_turn(session);
    if let Ok(mut sessions) = manager.sessions.lock() {
        let runtime = sessions.entry(session.to_string()).or_default();
        runtime.status = AdvisorStatus::Failed;
        runtime.last_error = Some(crate::message::redact_secrets(error));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_default_and_named_overrides_preserve_permissions() {
        let config = AdvisorConfig {
            enabled: true,
            allowed_runtime_keys: Some(vec!["openai-oauth".into()]),
            instructions: Some("Check requirements".into()),
            ..Default::default()
        };
        assert_eq!(entries(&config).unwrap()[0].name, "default");
        let config = AdvisorConfig {
            roster: vec![
                AdvisorRosterEntry {
                    name: "security".into(),
                    model: Some("reviewer".into()),
                    instructions: Some("Inspect authorization".into()),
                    ..Default::default()
                },
                AdvisorRosterEntry {
                    name: "verification".into(),
                    enabled: false,
                    ..Default::default()
                },
            ],
            ..config
        };
        let resolved = entries(&config).unwrap();
        assert_eq!(
            resolved[0].config.allowed_runtime_keys,
            config.allowed_runtime_keys
        );
        assert!(
            resolved[0]
                .instructions
                .contains("Check requirements\n\nInspect authorization")
        );
        assert_eq!(resolved[0].config.model.as_deref(), Some("reviewer"));
        assert!(!resolved[1].config.enabled);
        assert_ne!(
            runtime_session_key("session", "security"),
            runtime_session_key("session", "verification")
        );
    }
    #[test]
    fn duplicate_invalid_and_excessive_rosters_fail_visibly() {
        for names in [
            vec!["security", "security"],
            vec!["../escape"],
            vec![""],
            vec!["Uppercase"],
        ] {
            let config = AdvisorConfig {
                roster: names
                    .into_iter()
                    .map(|name| AdvisorRosterEntry {
                        name: name.into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
            assert!(entries(&config).is_err());
        }
        let config = AdvisorConfig {
            instructions: Some("x".repeat(MAX_INSTRUCTIONS_BYTES + 1)),
            ..Default::default()
        };
        assert!(entries(&config).is_err());
    }
}

#[cfg(test)]
mod runtime_tests;
