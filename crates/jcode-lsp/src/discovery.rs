use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use crate::{LspError, Result};

pub fn discover_executable(
    command: &str,
    path_env: Option<&OsStr>,
    working_dir: &Path,
) -> Result<PathBuf> {
    let command = command.trim();
    if command.is_empty() {
        return Err(LspError::ExecutableNotFound {
            command: command.to_owned(),
        });
    }

    let command_path = Path::new(command);
    if command_path.is_absolute()
        || command_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        let candidate = if command_path.is_absolute() {
            command_path.to_owned()
        } else {
            working_dir.join(command_path)
        };
        return checked_executable(candidate);
    }

    let path_env = path_env.ok_or_else(|| LspError::ExecutableNotFound {
        command: command.to_owned(),
    })?;
    for directory in std::env::split_paths(path_env) {
        for candidate in platform_candidates(&directory, command) {
            if is_executable_file(&candidate) {
                return candidate.canonicalize().map_err(LspError::from);
            }
        }
    }
    Err(LspError::ExecutableNotFound {
        command: command.to_owned(),
    })
}

fn checked_executable(candidate: PathBuf) -> Result<PathBuf> {
    if !is_executable_file(&candidate) {
        return Err(LspError::NotExecutable {
            path: candidate.display().to_string(),
        });
    }
    candidate.canonicalize().map_err(LspError::from)
}

fn platform_candidates(directory: &Path, command: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let extensions = std::env::var_os("PATHEXT")
            .map(|value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![".EXE".to_owned(), ".CMD".to_owned(), ".BAT".to_owned()]);
        let mut candidates = vec![directory.join(command)];
        if Path::new(command).extension().is_none() {
            candidates.extend(
                extensions
                    .into_iter()
                    .map(|extension| directory.join(format!("{command}{extension}"))),
            );
        }
        candidates
    }
    #[cfg(not(windows))]
    {
        vec![directory.join(command)]
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn make_executable(path: &Path) {
        fs::write(path, b"test").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn discovers_only_executable_files_without_shell_interpolation() {
        let root = std::env::temp_dir().join(format!("jcode-lsp-discovery-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let executable = root.join("rust-analyzer");
        make_executable(&executable);

        let found = discover_executable("rust-analyzer", Some(root.as_os_str()), &root).unwrap();
        assert_eq!(found, executable.canonicalize().unwrap());
        assert!(matches!(
            discover_executable("rust-analyzer --version", Some(root.as_os_str()), &root),
            Err(LspError::ExecutableNotFound { .. })
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_relative_paths_are_resolved_against_the_workspace() {
        let root = std::env::temp_dir().join(format!("jcode-lsp-explicit-{}", std::process::id()));
        let bin = root.join("bin");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&bin).unwrap();
        let executable = bin.join("server");
        make_executable(&executable);
        let found = discover_executable("./bin/server", None, &root).unwrap();
        assert_eq!(found, executable.canonicalize().unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_and_non_executable_paths_degrade_cleanly() {
        let root = std::env::temp_dir().join(format!("jcode-lsp-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("server"), b"not executable").unwrap();
        assert!(matches!(
            discover_executable("./server", None, &root),
            Err(LspError::NotExecutable { .. })
        ));
        assert!(matches!(
            discover_executable("missing", Some(root.as_os_str()), &root),
            Err(LspError::ExecutableNotFound { .. })
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
