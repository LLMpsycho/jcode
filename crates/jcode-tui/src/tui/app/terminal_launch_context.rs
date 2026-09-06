//! Resolve a child terminal's working directory and optional shared socket.
use std::path::PathBuf;

pub(super) fn resolve(session_id: &str) -> (PathBuf, Option<String>) {
    // A remote session may have no local snapshot. Keep the established current
    // directory fallback, while recording why the session directory was absent.
    let saved_dir = match crate::session::Session::load(session_id) {
        Ok(session) => session
            .working_dir
            .map(PathBuf::from)
            .filter(|path| path.is_dir()),
        Err(error) => {
            crate::logging::info(&format!(
                "Child terminal will use the current directory; session snapshot unavailable: {error}"
            ));
            None
        }
    };
    let cwd = saved_dir.unwrap_or_else(|| match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            crate::logging::warn(&format!(
                "Cannot resolve the child terminal working directory: {error}"
            ));
            PathBuf::from(".")
        }
    });
    let socket = match std::env::var("JCODE_SOCKET") {
        Ok(socket) => Some(socket),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            crate::logging::warn("Ignoring non-Unicode JCODE_SOCKET when opening a child terminal");
            None
        }
    };
    (cwd, socket)
}
