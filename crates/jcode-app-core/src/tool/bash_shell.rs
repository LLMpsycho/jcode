use super::*;

#[cfg(unix)]
pub(super) fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// Route ordinary `cargo` invocations (including those inside child scripts)
/// through the repository wrapper. Besides applying the project's build policy,
/// that wrapper appends real action timings to rust-actions.jsonl.
#[cfg(unix)]
pub(super) fn wrap_repo_cargo_commands(
    command: &str,
    working_dir: Option<&Path>,
) -> Option<String> {
    let working_dir = working_dir?;
    let repo = crate::build::find_repo_in_ancestors(working_dir)?;
    let wrapper = repo.join("scripts").join("dev_cargo.sh");
    if !wrapper.is_file() {
        return None;
    }

    Some(format!(
        r#"export JCODE_DEV_CARGO_SCRIPT={wrapper}
cargo() {{
  if [[ "${{JCODE_IN_DEV_CARGO:-0}}" == "1" ]]; then
    command cargo "$@"
  else
    JCODE_IN_DEV_CARGO=1 "$JCODE_DEV_CARGO_SCRIPT" "$@"
  fi
}}
export -f cargo
{command}"#,
        wrapper = shell_single_quote(&wrapper.to_string_lossy()),
    ))
}

/// Build a clear timeout message. The `timeout` param is in milliseconds, which
/// agents frequently mistake for seconds (e.g. passing 1000 thinking it means
/// 1000s when it is 1s). Spell out the seconds equivalent and, for suspiciously
/// short timeouts, hint that the unit is milliseconds so the next attempt uses a
/// sane value instead of repeating the same mistake.
pub(super) fn timeout_message(timeout_ms: u64) -> String {
    let secs = timeout_ms as f64 / 1000.0;
    let mut msg = format!("Command timed out after {}ms ({:.1}s)", timeout_ms, secs);
    if timeout_ms <= 5000 {
        msg.push_str(
            ". Note: the `timeout` parameter is in MILLISECONDS, not seconds. \
             If you meant a longer limit, pass a larger value (e.g. 600000 = 10min) or omit `timeout`.",
        );
    }
    msg
}

#[cfg(not(windows))]
pub(super) fn configure_tool_scratch(command: &mut TokioCommand) {
    if let Some(dir) = tool_scratch_dir() {
        command.env("TMPDIR", &dir).env("JCODE_SCRATCH_DIR", dir);
    }
}

pub(super) fn build_shell_command(cmd_str: &str) -> TokioCommand {
    #[cfg(windows)]
    {
        let mut cmd = TokioCommand::new("cmd.exe");
        // cmd.exe does not use the standard C runtime argument-decoding rules.
        // Passing the command through `arg` makes Rust escape nested quotes for
        // CommandLineToArgvW, which can corrupt commands such as:
        //
        //     gh issue create --title "text with spaces"
        //
        // Tokio's `raw_arg` is specifically provided for `cmd.exe /C`. Wrap the
        // full command in the outer quotes expected by cmd so its inner quotes
        // reach child programs intact. `/D` disables AutoRun hooks and `/S`
        // selects the documented quote handling used with this form.
        cmd.args(["/D", "/S", "/C"])
            .raw_arg(format!("\"{cmd_str}\""));
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = TokioCommand::new("bash");
        cmd.arg("-c").arg(cmd_str);
        configure_tool_scratch(&mut cmd);
        cmd
    }
}

pub(super) fn configure_background_command_stdio(command: &mut TokioCommand) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
}

#[cfg(unix)]
pub(super) fn build_detached_shell_wrapper(command: &str) -> StdCommand {
    let mut cmd = StdCommand::new("bash");
    cmd.arg("-lc")
        .arg(
            r#"eval "$JCODE_RELOAD_DETACH_COMMAND"; status=$?; printf '\n--- Command finished with exit code: %s ---\n' "$status"; exit "$status""#,
        )
        .env("JCODE_RELOAD_DETACH_COMMAND", command);
    if let Some(dir) = tool_scratch_dir() {
        cmd.env("TMPDIR", &dir).env("JCODE_SCRATCH_DIR", dir);
    }
    cmd
}

pub(super) fn format_command_output(mut output: String, exit_code: Option<i32>) -> String {
    if output.len() > MAX_OUTPUT_LEN {
        output = truncate_str(&output, MAX_OUTPUT_LEN).to_string();
        output.push_str("\n... (output truncated)");
    }

    if let Some(code) = exit_code.filter(|code| *code != 0) {
        output.push_str(&format!("\n\nExit code: {}", code));
    }

    if output.trim().is_empty() {
        "Command completed successfully (no output)".to_string()
    } else {
        output
    }
}
