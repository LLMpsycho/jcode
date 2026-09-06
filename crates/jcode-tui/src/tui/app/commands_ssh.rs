//! Interactive SSH remote setup and launch commands.
use super::*;

pub(in crate::tui::app) fn handle_ssh_command(app: &mut App, trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("/ssh") else {
        return false;
    };
    if !rest.is_empty()
        && !rest
            .chars()
            .next()
            .map(|c| c.is_whitespace())
            .unwrap_or(false)
    {
        return false;
    }

    let mut parts = rest.split_whitespace();
    let first = parts.next();
    match first {
        None => show_ssh_remotes(app),
        Some("add") => {
            let name = parts.next().unwrap_or("school");
            begin_ssh_target_prompt(app, name);
        }
        Some("status") => show_ssh_status(app),
        Some("disconnect") => {
            if let Some(name) = parts.next() {
                disconnect_ssh_remote(app, name);
            } else {
                app.push_display_message(DisplayMessage::error(
                    "Usage: /ssh disconnect <name>".to_string(),
                ));
            }
        }
        Some(name) => {
            let inline_target = parts.next();
            if let Some(target) = inline_target {
                match crate::ssh_remote::upsert_profile(name, target) {
                    Ok(profile) => connect_ssh_remote(app, profile),
                    Err(error) => app.push_display_message(DisplayMessage::error(format!(
                        "Failed to save SSH remote {}: {}",
                        name, error
                    ))),
                }
            } else {
                match crate::ssh_remote::find_profile(name) {
                    Ok(Some(profile)) => connect_ssh_remote(app, profile),
                    Ok(None) => begin_ssh_target_prompt(app, name),
                    Err(error) => app.push_display_message(DisplayMessage::error(format!(
                        "Failed to load SSH remotes: {}",
                        error
                    ))),
                }
            }
        }
    }
    true
}

pub(in crate::tui::app) fn handle_pending_ssh_remote_target(
    app: &mut App,
    name: String,
    input: String,
) {
    let target = input.trim();
    if target.is_empty() || target.eq_ignore_ascii_case("cancel") {
        app.push_display_message(DisplayMessage::system(
            "SSH remote setup cancelled.".to_string(),
        ));
        app.set_status_notice("SSH setup cancelled");
        return;
    }
    match crate::ssh_remote::upsert_profile(&name, target) {
        Ok(profile) => connect_ssh_remote(app, profile),
        Err(error) => app.push_display_message(DisplayMessage::error(format!(
            "Failed to save SSH remote {}: {}",
            name, error
        ))),
    }
}

fn begin_ssh_target_prompt(app: &mut App, name: &str) {
    app.pending_ssh_remote_name = Some(name.to_string());
    app.push_display_message(DisplayMessage::system(format!(
        "SSH setup: {}

Step 1/4: Tell Jcode where to connect.

Enter only the SSH target, meaning the part after ssh:

  alice@login.school.edu

You can also enter an SSH config alias like school.

Security model
  - Jcode stores this host/user target so you can run /ssh {} later.
  - Jcode does not ask for or store your SSH password.
  - If a password is needed, it will be typed into your system ssh prompt, not into Jcode.

Type cancel to stop setup.",
        name, name
    )));
    app.set_status_notice("SSH setup 1/4: enter target");
}

fn show_ssh_remotes(app: &mut App) {
    match crate::ssh_remote::load_config() {
        Ok(config) if config.hosts.is_empty() => {
            app.push_display_message(DisplayMessage::system(
                "SSH remotes

No SSH remotes are configured yet.

Start with:

  /ssh school

Jcode will ask for the SSH target, then use your system SSH client for authentication. Jcode never stores SSH passwords."
                    .to_string(),
            ));
        }
        Ok(config) => {
            let mut lines = vec!["SSH remotes".to_string(), "".to_string()];
            for profile in config.hosts {
                let alive = if crate::ssh_remote::is_control_master_alive(&profile) {
                    "✓ connected"
                } else {
                    "not connected"
                };
                lines.push(format!(
                    "  - {} -> {} ({})",
                    profile.name, profile.ssh_target, alive
                ));
            }
            lines.push("".to_string());
            lines.push(
                "Use /ssh <name> to connect, /ssh status to check, or /ssh disconnect <name> to disconnect."
                    .to_string(),
            );
            lines.push("".to_string());
            lines.push("Security: Jcode stores targets only, never SSH passwords.".to_string());
            app.push_display_message(DisplayMessage::system(lines.join("\n")));
        }
        Err(error) => app.push_display_message(DisplayMessage::error(format!(
            "Failed to load SSH remotes: {}",
            error
        ))),
    }
}

fn show_ssh_status(app: &mut App) {
    show_ssh_remotes(app);
}

fn connect_ssh_remote(app: &mut App, profile: crate::ssh_remote::SshRemoteProfile) {
    if crate::ssh_remote::is_control_master_alive(&profile)
        || crate::ssh_remote::can_connect_batch_mode(&profile)
    {
        app.push_display_message(DisplayMessage::system(format!(
            "SSH remote {}

Step 4/4: Connected.

Jcode verified that {} is reachable through your system SSH client.

What this means:
  - Authentication is handled by OpenSSH / your SSH agent.
  - Jcode did not see or store your password.
  - The SSH connection setup is ready for remote Jcode tools.

Next implementation step: start the remote Jcode server over this verified SSH connection.",
            profile.name, profile.ssh_target
        )));
        app.set_status_notice(format!("SSH {} connected 4/4", profile.name));
        return;
    }

    match crate::ssh_remote::spawn_control_master_terminal(&profile) {
        Ok(true) => {
            app.push_display_message(DisplayMessage::system(format!(
                "SSH remote {}

Step 2/4: Opening secure SSH login terminal.

Jcode could not connect without an interactive login, so it opened a separate terminal running your system ssh command.

What to expect in that terminal
  1. OpenSSH may ask for your password or two-factor prompt.
  2. You type credentials into OpenSSH, not into Jcode.
  3. After login, SSH creates a temporary background control socket.
  4. The terminal verifies that socket before closing.

Security model
  - Jcode cannot read what you type in the SSH terminal.
  - Jcode stores only the target {}.
  - Close or disconnect later with /ssh disconnect {}.",
                profile.name, profile.ssh_target, profile.name
            )));
            app.set_status_notice("SSH setup 2/4: login terminal opened");
        }
        Ok(false) => app.push_display_message(DisplayMessage::system(format!(
            "SSH remote {}

Step 2/4: Manual login needed.

Jcode could not open a terminal automatically. Run this command yourself:

  ssh -f -M -S {} -N {}

Type your password into that SSH prompt if asked. Jcode will not see or store it.",
            profile.name,
            crate::ssh_remote::control_socket_path(&profile.name)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "~/.jcode/ssh-control/remote.sock".to_string()),
            profile.ssh_target
        ))),
        Err(error) => app.push_display_message(DisplayMessage::error(format!(
            "Failed to open SSH login terminal: {}",
            error
        ))),
    }
}

fn disconnect_ssh_remote(app: &mut App, name: &str) {
    match crate::ssh_remote::find_profile(name) {
        Ok(Some(profile)) => match crate::ssh_remote::disconnect(&profile) {
            Ok(true) => {
                app.push_display_message(DisplayMessage::system(format!(
                    "Disconnected SSH remote {}.",
                    name
                )));
                app.set_status_notice("SSH disconnected");
            }
            Ok(false) => app.push_display_message(DisplayMessage::system(format!(
                "SSH remote {} did not have an active ControlMaster connection.",
                name
            ))),
            Err(error) => app.push_display_message(DisplayMessage::error(format!(
                "Failed to disconnect SSH remote {}: {}",
                name, error
            ))),
        },
        Ok(None) => app.push_display_message(DisplayMessage::error(format!(
            "Unknown SSH remote {}. Use /ssh to list remotes.",
            name
        ))),
        Err(error) => app.push_display_message(DisplayMessage::error(format!(
            "Failed to load SSH remote {}: {}",
            name, error
        ))),
    }
}
