use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use jcode_lsp::{LspConfig, LspServerConfig, Position};

use super::ToolContext;

pub(super) fn workspace_root(ctx: &ToolContext) -> Result<PathBuf> {
    ctx.working_dir
        .as_deref()
        .ok_or_else(|| anyhow!("lsp requires a session working directory"))?
        .canonicalize()
        .map_err(Into::into)
}

pub(super) fn resolve_file(root: &Path, file: &str) -> Result<PathBuf> {
    let path = root.join(file).canonicalize()?;
    if !path.starts_with(root) {
        bail!("LSP file must remain inside the workspace");
    }
    Ok(path)
}

pub(super) fn resolve_new_file(root: &Path, file: &str) -> Result<PathBuf> {
    let candidate = root.join(file);
    match std::fs::symlink_metadata(&candidate) {
        Ok(_) => bail!("LSP file rename destination already exists: {file}"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let parent = candidate
        .parent()
        .ok_or_else(|| anyhow!("LSP file rename destination has no parent"))?
        .canonicalize()?;
    if !parent.starts_with(root) {
        bail!("LSP file rename destination must remain inside the workspace");
    }
    let name = candidate
        .file_name()
        .ok_or_else(|| anyhow!("LSP file rename destination has no file name"))?;
    Ok(parent.join(name))
}

pub(super) fn select_server(
    config: &LspConfig,
    requested: Option<&str>,
    file: Option<&Path>,
) -> Result<String> {
    if let Some(requested) = requested {
        let server = config
            .servers
            .get(requested)
            .ok_or_else(|| anyhow!("LSP server `{requested}` is not configured"))?;
        if let Some(extension) = file
            .and_then(Path::extension)
            .and_then(|value| value.to_str())
            && !server.file_extensions.is_empty()
            && !server
                .file_extensions
                .iter()
                .any(|candidate| candidate == extension)
        {
            bail!("LSP server `{requested}` does not declare support for .{extension} files");
        }
        return Ok(requested.to_owned());
    }
    if let Some(extension) = file
        .and_then(Path::extension)
        .and_then(|value| value.to_str())
    {
        if let Some((server_id, _)) = config.servers.iter().find(|(_, server)| {
            server
                .file_extensions
                .iter()
                .any(|candidate| candidate == extension)
        }) {
            return Ok(server_id.clone());
        }
        bail!("no LSP server is configured for .{extension} files");
    }
    match config.servers.len() {
        0 => bail!("no LSP servers are configured"),
        1 => Ok(config.servers.keys().next().cloned().unwrap_or_default()),
        _ => bail!("this LSP action requires `server` when no file selects a language"),
    }
}

pub(super) fn discover_server_root(
    session_root: &Path,
    file: Option<&Path>,
    server: &LspServerConfig,
) -> PathBuf {
    let Some(mut cursor) = file.and_then(Path::parent) else {
        return session_root.to_owned();
    };
    loop {
        if server
            .root_markers
            .iter()
            .any(|marker| cursor.join(marker).exists())
        {
            return cursor.to_owned();
        }
        if cursor == session_root {
            break;
        }
        let Some(parent) = cursor.parent() else {
            break;
        };
        if !parent.starts_with(session_root) {
            break;
        }
        cursor = parent;
    }
    session_root.to_owned()
}

pub(super) fn language_id(path: &Path) -> &str {
    match path.extension().and_then(|value| value.to_str()) {
        Some("rs") => "rust",
        Some("ts") => "typescript",
        Some("tsx") => "typescriptreact",
        Some("js") => "javascript",
        Some("jsx") => "javascriptreact",
        Some("py") => "python",
        Some("go") => "go",
        _ => "plaintext",
    }
}

pub(super) fn one_based_position(line: Option<u32>, character: Option<u32>) -> Result<Position> {
    let line = line.ok_or_else(|| anyhow!("this LSP action requires `line`"))?;
    if line == 0 || character == Some(0) {
        bail!("LSP line and character values are one-based");
    }
    Ok(Position {
        line: line - 1,
        character: character.unwrap_or(1) - 1,
    })
}

#[cfg(test)]
mod tests {
    use super::resolve_new_file;

    #[test]
    fn new_file_resolution_rejects_existing_and_escaping_destinations() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir(workspace.path().join("src")).unwrap();
        std::fs::write(workspace.path().join("src/existing.rs"), "").unwrap();

        assert!(resolve_new_file(workspace.path(), "src/existing.rs").is_err());
        assert!(resolve_new_file(workspace.path(), "../escaping.rs").is_err());
        assert_eq!(
            resolve_new_file(workspace.path(), "src/new.rs").unwrap(),
            workspace.path().join("src/new.rs")
        );

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                workspace.path().join("missing-target"),
                workspace.path().join("src/dangling.rs"),
            )
            .unwrap();
            assert!(resolve_new_file(workspace.path(), "src/dangling.rs").is_err());
        }
    }
}
