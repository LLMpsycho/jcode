use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{
    DapError, DebugSessionSnapshot, DebugWorkspaceKey, InitializeRequestArguments, Result,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugAdapterKind {
    LldbDap,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugAdapterConfig {
    kind: DebugAdapterKind,
    adapter_id: String,
    executable: PathBuf,
}

impl DebugAdapterConfig {
    pub fn lldb_dap(executable: impl AsRef<Path>) -> Result<Self> {
        let input = executable.as_ref();
        if !input.is_absolute() {
            return Err(DapError::InvalidAdapterConfiguration {
                message: "adapter executable must be absolute".into(),
            });
        }
        let executable = input
            .canonicalize()
            .map_err(|e| DapError::AdapterUnavailable {
                path: input.to_path_buf(),
                message: e.to_string(),
            })?;
        validate_executable(&executable).map_err(|message| DapError::AdapterUnavailable {
            path: executable.clone(),
            message,
        })?;
        Ok(Self {
            kind: DebugAdapterKind::LldbDap,
            adapter_id: "lldb".into(),
            executable,
        })
    }
    pub fn kind(&self) -> DebugAdapterKind {
        self.kind
    }
    pub fn adapter_id(&self) -> &str {
        &self.adapter_id
    }
    pub fn executable(&self) -> &Path {
        &self.executable
    }
    pub(crate) fn revalidate(&self) -> Result<()> {
        let canonical =
            self.executable
                .canonicalize()
                .map_err(|error| DapError::AdapterUnavailable {
                    path: self.executable.clone(),
                    message: error.to_string(),
                })?;
        if canonical != self.executable {
            return Err(DapError::AdapterUnavailable {
                path: self.executable.clone(),
                message: "canonical adapter identity changed".to_owned(),
            });
        }
        validate_executable(&canonical).map_err(|message| DapError::AdapterUnavailable {
            path: self.executable.clone(),
            message,
        })
    }
}

macro_rules! request_type {
    ($name:ident $(, $field:ident : $ty:ty = $default:expr)?) => {
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name { program: PathBuf, args: Vec<String>, cwd: Option<PathBuf>, $($field: $ty,)? }
        impl $name {
            pub fn new(program: impl Into<PathBuf>) -> Self { Self { program: program.into(), args: vec![], cwd: None, $($field: $default,)? } }
            pub fn with_arg(mut self, arg: impl Into<String>) -> Self { self.args.push(arg.into()); self }
            pub fn with_args<I,S>(mut self, args: I) -> Self where I: IntoIterator<Item=S>, S: Into<String> { self.args.extend(args.into_iter().map(Into::into)); self }
            pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self { self.cwd=Some(cwd.into()); self }
            pub fn program(&self)->&Path { &self.program }
            pub fn args(&self)->&[String] { &self.args }
            pub fn cwd(&self)->Option<&Path> { self.cwd.as_deref() }
        }
    }
}
request_type!(DebugLaunchRequest, stop_on_entry: bool = false);
impl DebugLaunchRequest {
    pub fn with_stop_on_entry(mut self, stop: bool) -> Self {
        self.stop_on_entry = stop;
        self
    }
    pub fn stop_on_entry(&self) -> bool {
        self.stop_on_entry
    }
}
request_type!(DebugOwnedAttachRequest);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DebugSessionStart {
    Launch {
        program: PathBuf,
        cwd: PathBuf,
    },
    OwnedAttach {
        program: PathBuf,
        cwd: PathBuf,
        pid: u32,
    },
}

pub(crate) struct ResolvedProgram {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}
pub(crate) struct ResolvedLaunch {
    pub target: ResolvedProgram,
    pub stop_on_entry: bool,
}

pub(crate) fn resolve_program(
    workspace: &DebugWorkspaceKey,
    program: &Path,
    args: &[String],
    cwd: Option<&Path>,
) -> Result<ResolvedProgram> {
    if args.iter().any(|arg| arg.contains('\0')) {
        return Err(DapError::InvalidDebugProgram {
            path: program.to_path_buf(),
            message: "arguments must not contain NUL".into(),
        });
    }
    let root = workspace.canonical_root();
    let candidate = if program.is_absolute() {
        program.to_path_buf()
    } else {
        root.join(program)
    };
    let program = candidate
        .canonicalize()
        .map_err(|e| DapError::InvalidDebugProgram {
            path: candidate.clone(),
            message: e.to_string(),
        })?;
    if !program.starts_with(root) {
        return Err(DapError::DebugPathOutsideWorkspace {
            path: program,
            workspace: root.to_path_buf(),
        });
    }
    validate_program(&program)?;
    let cwd_input = cwd.map_or_else(
        || root.to_path_buf(),
        |p| {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            }
        },
    );
    let cwd = cwd_input
        .canonicalize()
        .map_err(|e| DapError::InvalidDebugWorkingDirectory {
            path: cwd_input.clone(),
            message: e.to_string(),
        })?;
    if !cwd.starts_with(root) {
        return Err(DapError::DebugPathOutsideWorkspace {
            path: cwd,
            workspace: root.to_path_buf(),
        });
    }
    if !cwd.is_dir() {
        return Err(DapError::InvalidDebugWorkingDirectory {
            path: cwd,
            message: "path is not a directory".into(),
        });
    }
    Ok(ResolvedProgram {
        program,
        args: args.to_vec(),
        cwd,
    })
}

pub(crate) fn revalidate_program(
    workspace: &DebugWorkspaceKey,
    target: &ResolvedProgram,
) -> Result<()> {
    let root = workspace.canonical_root();
    let program = target
        .program
        .canonicalize()
        .map_err(|error| DapError::InvalidDebugProgram {
            path: target.program.clone(),
            message: error.to_string(),
        })?;
    if program != target.program || !program.starts_with(root) {
        return Err(DapError::DebugPathOutsideWorkspace {
            path: program,
            workspace: root.to_path_buf(),
        });
    }
    validate_program(&program)?;
    let cwd =
        target
            .cwd
            .canonicalize()
            .map_err(|error| DapError::InvalidDebugWorkingDirectory {
                path: target.cwd.clone(),
                message: error.to_string(),
            })?;
    if cwd != target.cwd || !cwd.starts_with(root) {
        return Err(DapError::DebugPathOutsideWorkspace {
            path: cwd,
            workspace: root.to_path_buf(),
        });
    }
    if !target.cwd.is_dir() {
        return Err(DapError::InvalidDebugWorkingDirectory {
            path: target.cwd.clone(),
            message: "path is not a directory".to_owned(),
        });
    }
    Ok(())
}

pub(crate) enum AdapterProfile {
    LldbDap,
}
impl AdapterProfile {
    pub(crate) fn initialize_arguments(&self) -> InitializeRequestArguments {
        InitializeRequestArguments {
            client_id: Some("jcode".into()),
            client_name: Some("Jcode".into()),
            adapter_id: "lldb".into(),
            locale: None,
            lines_start_at1: Some(true),
            columns_start_at1: Some(true),
            path_format: Some("path".into()),
            supports_variable_type: None,
            supports_variable_paging: None,
            supports_run_in_terminal_request: Some(false),
        }
    }
    pub(crate) fn launch_arguments(&self, request: &ResolvedLaunch) -> Value {
        json!({"program":request.target.program,"args":request.target.args,"cwd":request.target.cwd,"stopOnEntry":request.stop_on_entry})
    }
    pub(crate) fn attach_arguments(&self, pid: u32) -> Value {
        json!({"pid":pid})
    }
}

fn validate_executable(path: &Path) -> std::result::Result<(), String> {
    let meta = path.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("path is not a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err("path is not executable".into());
        }
    }
    Ok(())
}
fn validate_program(path: &Path) -> Result<()> {
    let meta = path.metadata().map_err(|e| DapError::InvalidDebugProgram {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    if !meta.is_file() {
        return Err(DapError::InvalidDebugProgram {
            path: path.to_path_buf(),
            message: "path is not a regular file".into(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let mode = meta.permissions().mode();
        if mode & 0o111 == 0 {
            return Err(DapError::InvalidDebugProgram {
                path: path.to_path_buf(),
                message: "path is not executable".into(),
            });
        }
        if meta.mode() & 0o6000 != 0 {
            return Err(DapError::InvalidDebugProgram {
                path: path.to_path_buf(),
                message: "setuid and setgid programs are not allowed".into(),
            });
        }
    }
    Ok(())
}

// Keep public API references anchored in this module's contract.
const _: Option<fn() -> (DebugSessionSnapshot, DebugWorkspaceKey)> = None;

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod tests;
