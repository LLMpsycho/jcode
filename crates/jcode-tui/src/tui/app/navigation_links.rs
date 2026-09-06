//! Open transcript links and repository Markdown previews.
use super::*;

impl App {
    pub(in crate::tui::app) fn try_open_link_at(&mut self, column: u16, row: u16) -> bool {
        let Some(target) = crate::tui::ui::link_target_from_screen(column, row) else {
            return false;
        };

        if self.try_open_repository_markdown_link(&target) {
            return true;
        }

        match crate::tui::app::helpers::open_path_or_url_detached(&target) {
            Ok(()) => self.set_status_notice(format!("Opened link: {}", target)),
            Err(e) => self.set_status_notice(format!("Failed to open link: {}", e)),
        }
        true
    }

    pub(in crate::tui::app) fn try_open_repository_markdown_link(&mut self, target: &str) -> bool {
        if crate::tui::is_ssh_remote() {
            return false;
        }
        let path_target = target.split(['#', '?']).next().unwrap_or(target);
        let relative = std::path::Path::new(path_target);
        if relative.is_absolute()
            || path_target.contains("://")
            || !relative
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            return false;
        }

        let repository = match self.session.working_dir.as_deref() {
            Some(path) => std::path::PathBuf::from(path),
            None => match std::env::current_dir() {
                Ok(path) => path,
                Err(error) => {
                    self.set_status_notice(format!("Cannot locate the repository: {error}"));
                    return true;
                }
            },
        };
        let repository = match repository.canonicalize() {
            Ok(path) => path,
            Err(error) => {
                self.set_status_notice(format!("Cannot resolve the repository: {error}"));
                return true;
            }
        };
        let Ok(path) = repository.join(relative).canonicalize() else {
            self.set_status_notice(format!("Markdown file not found: {}", path_target));
            return true;
        };
        if !path.starts_with(&repository) {
            self.set_status_notice("Refused to open a Markdown file outside the repository");
            return true;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                self.set_status_notice(format!("Failed to read Markdown file: {}", error));
                return true;
            }
        };
        let title = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path_target)
            .to_string();
        let id = format!("linked-markdown:{}", path.display());
        let page = crate::side_panel::SidePanelPage {
            id: id.clone(),
            title: title.clone(),
            file_path: path.to_string_lossy().into_owned(),
            format: crate::side_panel::SidePanelPageFormat::Markdown,
            source: crate::side_panel::SidePanelPageSource::LinkedFile,
            content,
            updated_at_ms: 0,
        };

        let mut snapshot = self.side_panel.clone();
        if let Some(existing) = snapshot.pages.iter_mut().find(|existing| existing.id == id) {
            *existing = page;
        } else {
            snapshot.pages.push(page);
        }
        snapshot.focused_page_id = Some(id);
        self.side_panel_user_hidden = false;
        self.apply_side_panel_snapshot(snapshot);
        self.set_diff_pane_focus(true);
        self.set_status_notice(format!("Opened Markdown: {}", title));
        true
    }

    #[cfg(test)]
    pub(in crate::tui::app) fn try_open_link_at_with<F, E>(
        &mut self,
        column: u16,
        row: u16,
        mut open_url: F,
    ) -> bool
    where
        F: FnMut(&str) -> Result<(), E>,
        E: std::fmt::Display,
    {
        let Some(url) = crate::tui::ui::link_target_from_screen(column, row) else {
            return false;
        };

        match open_url(&url) {
            Ok(()) => self.set_status_notice(format!("Opened link: {}", url)),
            Err(e) => self.set_status_notice(format!("Failed to open link: {}", e)),
        }
        true
    }
}
