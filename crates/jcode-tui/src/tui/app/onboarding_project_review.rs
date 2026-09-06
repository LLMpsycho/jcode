//! Select and launch the first-run repository review.
use super::*;

impl App {
    /// Open the action-only onboarding choice. Session history remains available
    /// later through `/resume`, but first run stays focused on two clear paths.
    pub(in crate::tui::app) fn onboarding_open_start_choice(&mut self) {
        let mut picker = SessionPicker::new(Vec::new());
        picker.activate_onboarding_banner(Self::onboarding_start_choice_banner_lines());
        self.session_picker_overlay = Some(RefCell::new(picker));
        self.session_picker_mode = SessionPickerMode::Onboarding;
        self.set_onboarding_phase(OnboardingPhase::StartChoice {
            shown_at: Instant::now(),
        });
        self.onboarding_prefetch_recent_project();
        self.set_status_notice("Press any key to switch, Enter to choose");
    }

    /// Formatted copy shown above the two first-run actions.
    fn onboarding_start_choice_banner_lines() -> Vec<ratatui::text::Line<'static>> {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        let accent = crate::tui::color_support::rgb(186, 139, 255);
        vec![
            Line::from(vec![Span::styled(
                "Welcome to jcode 🎉",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![Span::styled(
                "How would you like to begin?",
                Style::default().fg(Color::White),
            )]),
        ]
    }

    /// Warm the most-active-project lookup while the user is still reading the start
    /// choice screen.
    ///
    /// Resolving the most active Git repository requires a full session-list
    /// scan, which is a cold multi-hundred-millisecond disk walk on machines
    /// with a large `~/.jcode/sessions` directory. Doing it inline on Enter made
    /// the suggested repository review action feel laggy, so run it
    /// off-thread as soon as the choice is displayed and have the key handler
    /// consume the cached answer.
    fn onboarding_prefetch_recent_project(&mut self) {
        if self.is_remote || self.onboarding_recent_project_prefetch.is_some() {
            return;
        }
        let slot: Arc<Mutex<Option<Option<PathBuf>>>> = Arc::new(Mutex::new(None));
        self.onboarding_recent_project_prefetch = Some(slot.clone());
        let session_id = self.session.id.clone();
        // A plain OS thread (not `tokio::spawn`) keeps this blocking filesystem
        // scan off the async runtime and works in tests without a reactor.
        std::thread::spawn(move || {
            let resolved = Self::recent_project_path_from_sessions(&session_id);
            if let Ok(mut slot) = slot.lock() {
                *slot = Some(resolved);
            }
        });
    }

    /// Resolve the user's most active project before the agent turn starts.
    /// Native and external session metadata are ranked by frequency and recency,
    /// keeping repository discovery out of the model prompt.
    pub(in crate::tui::app) fn onboarding_recent_project_path(&self) -> Option<PathBuf> {
        let home = dirs::home_dir();
        let excluded: Vec<PathBuf> = home.iter().cloned().collect();

        if self.is_remote
            && let Some(working_dir) = self.session.working_dir.as_deref()
        {
            let working_dir = Path::new(working_dir);
            // Remote session history is not available on the client. Trust the
            // server-provided directory, but never review the bare home directory.
            if !working_dir.as_os_str().is_empty()
                && home.as_deref().is_none_or(|home| home != working_dir)
            {
                return Some(working_dir.to_path_buf());
            }
        }

        if self.is_remote {
            return None;
        }

        // Prefer the prefetched answer so the action responds immediately.
        if let Some(repository) = self
            .onboarding_recent_project_prefetch
            .as_ref()
            .and_then(|slot| match slot.lock() {
                Ok(slot) => slot.clone(),
                Err(error) => {
                    crate::logging::warn(&format!("Recent-project prefetch unavailable: {error}"));
                    None
                }
            })
            .flatten()
        {
            return Some(repository);
        }

        if let Some(repository) = Self::recent_project_path_from_sessions(&self.session.id) {
            return Some(repository);
        }

        // A brand-new user has no history to rank yet. In that case only, use
        // the repository they launched jcode from rather than disabling review.
        self.session
            .working_dir
            .as_deref()
            .and_then(|working_dir| repo_ranking::resolve_git_root(Path::new(working_dir)))
            .filter(|root| !excluded.iter().any(|excluded| excluded == root))
    }

    /// Most active Git repository across recorded sessions, excluding the
    /// current session and the bare home directory. Blocking: this walks the
    /// session list on disk.
    fn recent_project_path_from_sessions(current_session_id: &str) -> Option<PathBuf> {
        let excluded: Vec<PathBuf> = dirs::home_dir().into_iter().collect();
        let sessions = match load_sessions() {
            Ok(sessions) => sessions,
            Err(error) => {
                crate::logging::warn(&format!("Cannot load recent project history: {error}"));
                return None;
            }
        };
        let locations: Vec<SessionLocation> = sessions
            .into_iter()
            .filter(|session| {
                session.id != current_session_id && !session.is_debug && !session.is_canary
            })
            .filter_map(|session| {
                let working_dir = session.working_dir?;
                Some(SessionLocation::new(
                    working_dir,
                    session.last_active_at.or(Some(session.last_message_time)),
                ))
            })
            .collect();
        let options = repo_ranking::RankOptions {
            excluded_paths: excluded,
            ..repo_ranking::RankOptions::default()
        };
        repo_ranking::rank_repositories(&locations, Utc::now(), &options)
            .into_iter()
            .next()
            .map(|repo| PathBuf::from(repo.path))
    }

    /// First-turn prompt launched by the onboarding recent-project review action.
    /// Repository discovery is deliberately absent: the path has already been
    /// selected programmatically before this prompt is built.
    pub(in crate::tui::app) fn onboarding_recent_project_review_prompt(
        repository: &Path,
    ) -> String {
        let repository = format!("{:?}", repository.to_string_lossy());
        format!(
            "Find the most critical architecture problems in the repository at {repository}. Do not fix them yet, and ask me whether I want them fixed once you find them."
        )
    }

    pub(in crate::tui::app) fn onboarding_prepare_recent_project_review(&mut self) -> bool {
        let Some(repository) = self.onboarding_recent_project_path() else {
            self.onboarding_show_suggestions();
            self.set_status_notice(
                "No active Git repository found. Start jcode inside a project to review it.",
            );
            return false;
        };
        self.onboarding_finish();
        self.input = Self::onboarding_recent_project_review_prompt(&repository);
        self.cursor_pos = self.input.len();
        true
    }

    /// Start the proactive recent-project review on the active runtime.
    ///
    /// Local TUIs consume `pending_turn` in their run loop, while remote-attached
    /// TUIs send queued messages from the remote tick loop. Calling
    /// [`App::submit_input`] in remote mode leaves the client permanently parked
    /// in `Sending` because no local run loop exists to consume that flag.
    pub(in crate::tui::app) fn onboarding_start_recent_project_review(&mut self) {
        if !self.onboarding_prepare_recent_project_review() {
            return;
        }
        self.follow_chat_bottom_for_typing();
        if self.is_remote {
            crate::tui::app::input::queue_message(self);
            self.set_status_notice("Architecture review queued");
        } else {
            self.submit_input();
        }
    }
}
