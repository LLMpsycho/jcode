use super::*;

pub(super) async fn login_google_flow(
    no_browser: bool,
    access_tier: Option<auth::google::GmailAccessTier>,
) -> Result<()> {
    use auth::google::{GmailAccessTier, GoogleCredentials};

    eprintln!("╔══════════════════════════════════════════╗");
    eprintln!("║       Gmail Integration Setup            ║");
    eprintln!("╚══════════════════════════════════════════╝\n");

    let _creds = match auth::google::load_credentials() {
        Ok(creds) => {
            eprintln!(
                "✓ Google credentials found (client_id: {}...)\n",
                &creds.client_id[..20.min(creds.client_id.len())]
            );
            creds
        }
        Err(_) => {
            eprintln!("No Google credentials found. Let's set them up.\n");
            eprintln!("You need OAuth credentials from Google Cloud Console.");
            eprintln!("How would you like to provide them?\n");
            eprintln!("  [1] Paste client ID and secret directly (easiest)");
            eprintln!("  [2] Provide path to downloaded JSON credentials file");
            eprintln!("  [3] I need help creating credentials (opens setup guide)\n");
            eprint!("Choose [1/2/3]: ");
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            match input.trim() {
                "1" => {
                    eprintln!("\nPaste your Google OAuth Client ID:");
                    eprintln!("  (looks like: 123456789-abc.apps.googleusercontent.com)\n");
                    eprint!("> ");
                    io::stdout().flush()?;
                    let mut client_id = String::new();
                    io::stdin().read_line(&mut client_id)?;
                    let client_id = client_id.trim().to_string();

                    if client_id.is_empty() {
                        anyhow::bail!("No client ID provided.");
                    }

                    eprintln!("\nPaste your Google OAuth Client Secret:");
                    eprintln!("  (looks like: GOCSPX-...)\n");
                    eprint!("> ");
                    io::stdout().flush()?;
                    let mut client_secret = String::new();
                    io::stdin().read_line(&mut client_secret)?;
                    let client_secret = client_secret.trim().to_string();

                    if client_secret.is_empty() {
                        anyhow::bail!("No client secret provided.");
                    }

                    let creds = GoogleCredentials {
                        client_id,
                        client_secret,
                    };
                    auth::google::save_credentials(&creds)?;
                    eprintln!(
                        "\n✓ Credentials saved to {}\n",
                        auth::google::credentials_path()?.display()
                    );
                    creds
                }
                "2" => {
                    eprintln!("\nPaste the path to your downloaded JSON file:\n");
                    eprint!("> ");
                    io::stdout().flush()?;
                    let mut path_input = String::new();
                    io::stdin().read_line(&mut path_input)?;
                    let path_str = path_input.trim();

                    let path_str = if let Some(stripped) = path_str.strip_prefix("~/") {
                        if let Some(home) = dirs::home_dir() {
                            home.join(stripped).to_string_lossy().to_string()
                        } else {
                            path_str.to_string()
                        }
                    } else {
                        path_str.to_string()
                    };

                    let data = std::fs::read_to_string(&path_str)
                        .with_context(|| format!("Could not read file: {}", path_str))?;

                    let dest = auth::google::credentials_path()?;
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                        crate::platform::set_directory_permissions_owner_only(parent)?;
                    }
                    std::fs::write(&dest, &data)?;
                    crate::platform::set_permissions_owner_only(&dest)?;

                    let creds = auth::google::load_credentials()
                        .context("Could not parse the credentials file. Make sure it's the OAuth client JSON from Google Cloud Console.")?;

                    eprintln!("\n✓ Credentials imported to {}\n", dest.display());
                    creds
                }
                "3" => {
                    eprintln!("\n── Step-by-step Google Cloud setup ──\n");

                    eprintln!("1. Open Google Cloud Console and create a project:");
                    eprintln!("   Opening: https://console.cloud.google.com/projectcreate\n");
                    maybe_open_browser(
                        "https://console.cloud.google.com/projectcreate",
                        no_browser,
                    );
                    eprint!("   Press Enter when your project is created...");
                    io::stdout().flush()?;
                    let mut wait = String::new();
                    io::stdin().read_line(&mut wait)?;

                    eprintln!("\n2. Enable the Gmail API:");
                    eprintln!("   Opening: Gmail API library page\n");
                    maybe_open_browser(
                        "https://console.cloud.google.com/apis/library/gmail.googleapis.com",
                        no_browser,
                    );
                    eprintln!("   Click the blue 'Enable' button.");
                    eprint!("   Press Enter when done...");
                    io::stdout().flush()?;
                    io::stdin().read_line(&mut wait)?;

                    eprintln!("\n3. Configure OAuth consent screen:");
                    eprintln!("   Opening: OAuth consent screen\n");
                    maybe_open_browser(
                        "https://console.cloud.google.com/apis/credentials/consent",
                        no_browser,
                    );
                    eprintln!("   - Choose 'External' user type");
                    eprintln!("   - Fill in app name (e.g. 'jcode') and your email");
                    eprintln!("   - Skip scopes (we'll request them during login)");
                    eprintln!("   - Add your email as a test user");
                    eprintln!("   - Save and continue through all steps");
                    eprint!("   Press Enter when done...");
                    io::stdout().flush()?;
                    io::stdin().read_line(&mut wait)?;

                    eprintln!("\n4. Create OAuth credentials:");
                    eprintln!("   Opening: Credentials page\n");
                    maybe_open_browser(
                        "https://console.cloud.google.com/apis/credentials",
                        no_browser,
                    );
                    eprintln!("   - Click '+ Create Credentials' > 'OAuth client ID'");
                    eprintln!("   - Application type: 'Desktop app'");
                    eprintln!("   - Name: 'jcode'");
                    eprintln!("   - Click 'Create'\n");
                    eprintln!("   A dialog will show your Client ID and Client Secret.\n");

                    eprintln!("Paste your Client ID:");
                    eprint!("> ");
                    io::stdout().flush()?;
                    let mut client_id = String::new();
                    io::stdin().read_line(&mut client_id)?;
                    let client_id = client_id.trim().to_string();

                    if client_id.is_empty() {
                        anyhow::bail!("No client ID provided.");
                    }

                    eprintln!("\nPaste your Client Secret:");
                    eprint!("> ");
                    io::stdout().flush()?;
                    let mut client_secret = String::new();
                    io::stdin().read_line(&mut client_secret)?;
                    let client_secret = client_secret.trim().to_string();

                    if client_secret.is_empty() {
                        anyhow::bail!("No client secret provided.");
                    }

                    let creds = GoogleCredentials {
                        client_id,
                        client_secret,
                    };
                    auth::google::save_credentials(&creds)?;
                    eprintln!("\n✓ Credentials saved!\n");
                    creds
                }
                _ => {
                    eprintln!("\nInvalid choice. Please enter 1, 2, or 3.\n");
                    std::process::exit(1);
                }
            }
        }
    };

    let tier = if let Some(tier) = access_tier {
        tier
    } else {
        eprintln!("── Gmail Access Level ──\n");
        eprintln!("  [1] Full Access (recommended)");
        eprintln!("      Search, read, draft, send, and manage emails.");
        eprintln!("      Send and delete always require your confirmation.\n");
        eprintln!("  [2] Read & Draft Only");
        eprintln!("      Search, read emails, create drafts. Cannot send or delete.");
        eprintln!("      API-level restriction - impossible even if the AI tries.\n");
        eprint!("Choose [1/2] (default: 1): ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        match input.trim() {
            "" | "1" => GmailAccessTier::Full,
            "2" => GmailAccessTier::ReadOnly,
            _ => {
                eprintln!("Invalid choice, defaulting to Full Access.");
                GmailAccessTier::Full
            }
        }
    };

    eprintln!("\nAccess level: {}", tier.label());

    eprintln!("\n── Logging in ──\n");

    let tokens = auth::google::login(tier, no_browser).await?;

    eprintln!("\n╔══════════════════════════════════════════╗");
    eprintln!("║  ✓ Gmail setup complete!                 ║");
    eprintln!("╚══════════════════════════════════════════╝\n");
    if let Some(email) = &tokens.email {
        eprintln!("  Account:      {}", email);
    }
    eprintln!("  Access tier:  {}", tokens.tier.label());
    eprintln!(
        "  Credentials:  {}",
        auth::google::credentials_path()?.display()
    );
    eprintln!(
        "  Tokens:       {}\n",
        auth::google::tokens_path()?.display()
    );
    eprintln!("The 'gmail' tool is enabled by default in the full tool profile.");
    eprintln!("To hide it, add `disabled = [\"gmail\"]` to [tools] in config.toml.");
    eprintln!("Then try asking: \"check my recent emails\" or \"search emails from ...\"");

    crate::telemetry::record_auth_success("google", "oauth");
    Ok(())
}
