//! Named swarm specializations from global and workspace-local Markdown files.

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AgentProfile {
    pub name: String,
    pub description: String,
    pub content: String,
    pub allowed_tools: Option<Vec<String>>,
    pub effort: Option<String>,
    pub path: PathBuf,
}

impl AgentProfile {
    pub fn display_name(&self) -> String {
        format!("{} agent", self.name.replace('-', " "))
    }
}

#[derive(Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
    #[serde(default, rename = "allowed-tools", alias = "tools")]
    allowed_tools: Option<Tools>,
    #[serde(default, alias = "thinking")]
    effort: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Tools {
    List(Vec<String>),
    CommaSeparated(String),
}

fn parse_profile(path: &Path) -> Result<AgentProfile> {
    const MAX_PROFILE_BYTES: u64 = 128 * 1024;
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_PROFILE_BYTES + 1)
        .read_to_string(&mut text)?;
    ensure!(
        text.len() as u64 <= MAX_PROFILE_BYTES,
        "Profile exceeds 128 KiB"
    );
    let text = text.replace("\r\n", "\n");
    let (yaml, content) = text
        .trim_start_matches('\u{feff}')
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .context("Expected YAML frontmatter followed by a Markdown prompt")?;
    let metadata: Frontmatter = serde_yaml::from_str(yaml)?;
    let name = metadata.name;
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && !name.starts_with('-')
            && !name.ends_with('-')
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
        "Profile name must be a lowercase identifier of up to 64 letters, digits, or hyphens"
    );
    let description = metadata
        .description
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    ensure!(
        !description.is_empty() && description.len() <= 2048,
        "Profile needs a description of up to 2048 bytes"
    );
    ensure!(!content.trim().is_empty(), "Profile prompt is empty");
    let allowed_tools = metadata.allowed_tools.map(|tools| {
        let tools = match tools {
            Tools::List(tools) => tools,
            Tools::CommaSeparated(tools) => tools.split(',').map(str::to_string).collect(),
        };
        tools
            .into_iter()
            .map(|tool| match tool.trim() {
                "grep" | "find" => "agentgrep".to_string(),
                tool => tool.to_string(),
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    });
    if let Some(tools) = &allowed_tools {
        ensure!(
            tools.iter().all(|tool| !tool.is_empty()
                && tool
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))),
            "Invalid tool name in profile"
        );
    }
    let effort = metadata.effort.map(|effort| effort.trim().to_string());
    if let Some(effort) = &effort {
        ensure!(
            matches!(
                effort.as_str(),
                "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
            ),
            "Invalid profile reasoning effort"
        );
    }
    Ok(AgentProfile {
        name,
        description,
        content: content.trim().to_string(),
        allowed_tools,
        effort,
        path: path.to_path_buf(),
    })
}

fn load_directories(directories: impl IntoIterator<Item = PathBuf>) -> Result<Vec<AgentProfile>> {
    let mut profiles = BTreeMap::new();
    for directory in directories {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Read agent profiles in {}", directory.display()));
            }
        };
        let mut paths = entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        let mut names = BTreeSet::new();
        for path in paths
            .into_iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        {
            let profile = parse_profile(&path)
                .with_context(|| format!("Read agent profile {}", path.display()))?;
            ensure!(
                names.insert(profile.name.clone()),
                "Duplicate agent profile {} in {}",
                profile.name,
                directory.display()
            );
            profiles.insert(profile.name.clone(), profile);
        }
    }
    Ok(profiles.into_values().collect())
}

/// Workspace definitions override global definitions by name. Never consult the
/// daemon's current directory for a session that has no workspace.
pub fn load_profiles(working_dir: Option<&Path>) -> Result<Vec<AgentProfile>> {
    let mut directories = vec![crate::storage::jcode_dir()?.join("agents")];
    if let Some(working_dir) = working_dir {
        directories.push(working_dir.join(".jcode/agents"));
    }
    load_directories(directories)
}

pub fn resolve_profile(name: &str, working_dir: Option<&Path>) -> Result<AgentProfile> {
    if let Some(profile) = load_profiles(working_dir)?
        .into_iter()
        .find(|profile| profile.name == name)
    {
        return Ok(profile);
    }
    bail!("Unknown agent profile {name:?}; see /agents for available profiles")
}

/// Metadata only: workers load the full prompt when a profile is selected.
pub fn catalog_prompt(working_dir: Option<&Path>) -> Result<String> {
    let profiles = load_profiles(working_dir)?;
    if profiles.is_empty() {
        return Ok(String::new());
    }
    let mut prompt = String::from(
        "# Available agent profiles\n\nChoose a matching specialization when delegating with swarm spawn or assign_task. Pass `profile` with the exact identifier and a concrete task prompt. Profiles add role instructions and tool restrictions, retain the configured swarm model, and use their role name instead of a random name. Do not spawn agents just because profiles exist.\n",
    );
    for profile in profiles {
        prompt.push_str(&format!(
            "\n- `{}` ({}): {}",
            profile.name,
            profile.display_name(),
            profile.description
        ));
    }
    Ok(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_profile(directory: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(directory.join(format!("{name}.md")), format!("---\nname: {name}\ndescription: Use for {name} tasks\ntools: read, grep, find\nthinking: high\n---\n{body}\n")).unwrap();
    }

    #[test]
    fn profiles_parse_omp_metadata_and_overlay_by_workspace() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global");
        let project = root.path().join("project");
        write_profile(&global, "devops", "Global infrastructure instructions");
        write_profile(&global, "debug", "Diagnose before fixing");
        write_profile(&project, "devops", "Project infrastructure instructions");
        let profiles = load_directories([global, project]).unwrap();
        assert_eq!(
            profiles
                .iter()
                .map(|profile| profile.name.as_str())
                .collect::<Vec<_>>(),
            ["debug", "devops"]
        );
        assert_eq!(profiles[1].content, "Project infrastructure instructions");
        assert_eq!(profiles[1].display_name(), "devops agent");
        assert_eq!(
            profiles[1].allowed_tools.as_deref().unwrap(),
            ["agentgrep", "read"]
        );
        assert_eq!(profiles[1].effort.as_deref(), Some("high"));
        assert!(
            load_directories([root.path().join("missing")])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn profiles_reject_invalid_metadata_and_duplicate_names() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("profile.md");
        for text in [
            "no frontmatter",
            "---\nname: ../devops\ndescription: Bad name\n---\nInstructions",
            "---\nname: devops\ndescription: Test\n---\n   ",
            "---\nname: devops\ndescription: Test\ntools: ['*']\n---\nInstructions",
            "---\nname: devops\ndescription: Test\nthinking: swarm-deep\n---\nInstructions",
        ] {
            std::fs::write(&path, text).unwrap();
            assert!(parse_profile(&path).is_err(), "accepted {text}");
        }
        write_profile(root.path(), "devops", "Instructions");
        std::fs::copy(root.path().join("devops.md"), &path).unwrap();
        assert!(load_directories([root.path().to_path_buf()]).is_err());
    }
}
