use super::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const MAX_PROJECT_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProjectAdvisor {
    instructions: Option<String>,
    roster: Vec<AdvisorRosterEntry>,
}

struct CachedProject {
    working_dir: PathBuf,
    checked: Instant,
    result: std::result::Result<ProjectAdvisor, String>,
}

static PROJECTS: LazyLock<Mutex<HashMap<String, CachedProject>>> = LazyLock::new(Mutex::default);

/// Project files can specialize advisors but cannot enable them globally,
/// expand tool permissions, relax budgets, or widen authenticated route grants.
/// Cache the tiny project document for one second, bounding step-path disk I/O.
pub fn config_for_owner(
    owner: &str,
    global: &AdvisorConfig,
    working_dir: Option<&Path>,
) -> Result<AdvisorConfig> {
    let mut projects = PROJECTS
        .lock()
        .map_err(|_| anyhow::anyhow!("advisor project configuration unavailable"))?;
    let dir = working_dir
        .map(Path::to_path_buf)
        .or_else(|| projects.get(owner).map(|entry| entry.working_dir.clone()));
    let Some(dir) = dir else {
        return Ok(global.clone());
    };
    let fresh = projects.get(owner).is_some_and(|entry| {
        entry.working_dir == dir && entry.checked.elapsed() < Duration::from_secs(1)
    });
    if !fresh {
        let result = load(&dir).map_err(|error| error.to_string());
        if projects.len() >= 128 && !projects.contains_key(owner) {
            projects.clear();
        }
        projects.insert(
            owner.into(),
            CachedProject {
                working_dir: dir,
                checked: Instant::now(),
                result,
            },
        );
    }
    let project = projects
        .get(owner)
        .ok_or_else(|| anyhow::anyhow!("advisor project configuration unavailable"))?
        .result
        .clone()
        .map_err(anyhow::Error::msg)?;
    merge(global, project)
}

fn load(working_dir: &Path) -> Result<ProjectAdvisor> {
    let root = working_dir
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("advisor working directory is unavailable"))?;
    let path = root.join(".jcode/advisor.toml");
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProjectAdvisor::default());
        }
        Err(_) => bail!("advisor project configuration cannot be inspected"),
    };
    ensure!(
        metadata.is_file(),
        "advisor project configuration must be a regular file"
    );
    let canonical = path
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("advisor project configuration cannot be resolved"))?;
    ensure!(
        canonical.starts_with(&root),
        "advisor project configuration must remain inside the workspace"
    );
    let file = std::fs::File::open(canonical)
        .map_err(|_| anyhow::anyhow!("advisor project configuration cannot be read"))?;
    ensure!(
        file.metadata()?.len() <= MAX_PROJECT_BYTES,
        "advisor project configuration exceeds 64 KiB"
    );
    let mut text = String::new();
    file.take(MAX_PROJECT_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| anyhow::anyhow!("advisor project configuration must contain UTF-8 text"))?;
    ensure!(
        text.len() as u64 <= MAX_PROJECT_BYTES,
        "advisor project configuration exceeds 64 KiB"
    );
    // Parsing errors can quote source lines containing secrets; expose only a
    // stable actionable error, never the deserializer's source excerpt.
    toml::from_str(&text).map_err(|_| {
        anyhow::anyhow!("invalid .jcode/advisor.toml; expected instructions and [[roster]] entries")
    })
}

fn merge(global: &AdvisorConfig, project: ProjectAdvisor) -> Result<AdvisorConfig> {
    let project_config = AdvisorConfig {
        instructions: project.instructions.clone(),
        roster: project.roster.clone(),
        ..Default::default()
    };
    entries(&project_config)?;
    entries(global)?;
    let mut merged = global.clone();
    if let Some(instructions) = project.instructions {
        let shared = merged.instructions.get_or_insert_default();
        if !shared.is_empty() {
            shared.push_str("\n\n");
        }
        shared.push_str(&instructions);
    }
    for entry in project.roster {
        if let Some(existing) = merged
            .roster
            .iter_mut()
            .find(|existing| existing.name == entry.name)
        {
            *existing = entry;
        } else {
            merged.roster.push(entry);
        }
    }
    entries(&merged)?;
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_overrides_names_without_relaxing_global_controls() {
        let global = AdvisorConfig {
            enabled: false,
            allowed_runtime_keys: Some(vec![]),
            max_reviews_per_session: 7,
            instructions: Some("Global".into()),
            roster: vec![AdvisorRosterEntry {
                name: "security".into(),
                model: Some("old".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let project: ProjectAdvisor = toml::from_str("instructions = 'Project'\n[[roster]]\nname = 'security'\nmodel = 'new'\n[[roster]]\nname = 'verification'\neffort = 'high'\n").unwrap();
        let merged = merge(&global, project).unwrap();
        assert!(!merged.enabled);
        assert_eq!(merged.allowed_runtime_keys, Some(vec![]));
        assert_eq!(merged.max_reviews_per_session, 7);
        assert_eq!(merged.roster.len(), 2);
        assert_eq!(merged.roster[0].model.as_deref(), Some("new"));
        assert_eq!(merged.instructions.as_deref(), Some("Global\n\nProject"));
        assert!(toml::from_str::<ProjectAdvisor>("enabled = true").is_err());
        assert!(
            toml::from_str::<ProjectAdvisor>("allowed_runtime_keys = ['openai-api-key']").is_err()
        );
    }
    #[test]
    fn project_file_boundaries_and_parser_errors_are_safe() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(".jcode")).unwrap();
        let path = temp.path().join(".jcode/advisor.toml");
        std::fs::write(&path, "instructions = 'unterminated_secret").unwrap();
        assert!(
            !load(temp.path())
                .unwrap_err()
                .to_string()
                .contains("unterminated_secret")
        );
        std::fs::write(&path, "x".repeat(MAX_PROJECT_BYTES as usize + 1)).unwrap();
        assert!(load(temp.path()).is_err());
        #[cfg(unix)]
        {
            std::fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(temp.path().join("external"), &path).unwrap();
            assert!(load(temp.path()).is_err());
        }
    }
}
