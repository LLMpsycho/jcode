use super::*;

/// Lines for the pinned status band: optional todos followed by exactly one
/// compact row per relevant background task. Completed tasks are shown briefly
/// as confirmation, while running and failed tasks remain actionable.
pub(super) fn pinned_todo_band_lines(
    app: &dyn TuiState,
    width: u16,
    viewport_height: u16,
) -> (Vec<Line<'static>>, Option<usize>) {
    if width < 16 || viewport_height < 3 {
        return (Vec::new(), None);
    }

    let task_lines: Vec<_> = app
        .background_task_rows()
        .iter()
        .map(|task| active_background_task_line(task, width))
        .collect();
    let card_lines = if crate::config::config().display.pin_todos {
        app.pinned_todos_payload().map_or_else(Vec::new, |payload| {
            let msg = crate::tui::DisplayMessage::todos(payload.to_string());
            super::super::messages::get_cached_message_lines(
                &msg,
                width,
                app.diff_mode(),
                super::super::messages::render_todos_message,
            )
        })
    } else {
        Vec::new()
    };
    if card_lines.is_empty() && task_lines.is_empty() {
        return (Vec::new(), None);
    }

    // Band budget: about a third of the viewport.
    let budget = ((viewport_height as usize) / 3).clamp(2, 12);
    let content_budget = budget.saturating_sub(task_lines.len()).max(2);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let has_more = card_lines.len() > content_budget && !app.pinned_todos_expanded();
    let mut more_line = None;
    if has_more {
        let shown = content_budget.saturating_sub(1);
        let hidden = card_lines.len() - shown;
        lines.extend(card_lines.into_iter().take(shown));
        more_line = Some(lines.len());
        lines.push(Line::from(Span::styled(
            format!("  … +{} more (todo)", hidden),
            Style::default().fg(dim_color()),
        )));
    } else {
        lines.extend(card_lines);
    }
    lines.extend(task_lines);
    (lines, more_line)
}

fn active_background_task_line(task: &crate::tui::BackgroundTaskRow, width: u16) -> Line<'static> {
    const BAR_WIDTH: usize = 6;
    let (icon, task_color, percent) = match task.status {
        crate::tui::BackgroundTaskRowStatus::Running => (
            "◌",
            accent_color(),
            task.percent.unwrap_or(0.0).clamp(0.0, 100.0),
        ),
        crate::tui::BackgroundTaskRowStatus::Completed => ("✓", Color::Green, 100.0),
        crate::tui::BackgroundTaskRowStatus::Failed => (
            "×",
            Color::Red,
            task.percent.unwrap_or(0.0).clamp(0.0, 100.0),
        ),
    };
    let rounded_percent = percent.round() as u8;
    let status_label = if task.status == crate::tui::BackgroundTaskRowStatus::Failed {
        "failed".to_string()
    } else {
        format!("{}%", rounded_percent)
    };
    let filled = ((percent / 100.0) * BAR_WIDTH as f32).round() as usize;
    let (active_bar, remaining_bar) = if task.status == crate::tui::BackgroundTaskRowStatus::Failed
    {
        (
            "━".repeat(filled.min(BAR_WIDTH)),
            "─".repeat(BAR_WIDTH.saturating_sub(filled)),
        )
    } else if filled >= BAR_WIDTH {
        ("━".repeat(BAR_WIDTH), String::new())
    } else {
        (
            format!("{}╺", "━".repeat(filled)),
            "─".repeat(BAR_WIDTH.saturating_sub(filled + 1)),
        )
    };

    let fixed_width = UnicodeWidthStr::width(
        format!("◌ bg   {} {}{}", active_bar, remaining_bar, status_label).as_str(),
    );
    let max_label_width = (width as usize).saturating_sub(fixed_width).max(1);
    let label = truncate_background_task_label(&task.label, max_label_width);

    Line::from(vec![
        Span::styled(icon, Style::default().fg(task_color)),
        Span::styled(" bg ", Style::default().fg(dim_color())),
        Span::raw(label),
        Span::raw("  "),
        Span::styled(active_bar, Style::default().fg(task_color)),
        Span::styled(remaining_bar, Style::default().fg(dim_color())),
        Span::styled(
            format!(" {}", status_label),
            Style::default().fg(dim_color()),
        ),
    ])
}

fn truncate_background_task_label(label: &str, max_width: usize) -> String {
    let label = label.replace(['\r', '\n'], " ");
    if UnicodeWidthStr::width(label.as_str()) <= max_width {
        return label;
    }
    if max_width <= 1 {
        return "…".to_string();
    }
    let mut truncated = String::new();
    for ch in label.chars() {
        let candidate = format!("{}{}…", truncated, ch);
        if UnicodeWidthStr::width(candidate.as_str()) > max_width {
            break;
        }
        truncated.push(ch);
    }
    truncated.push('…');
    truncated
}

static PINNED_TODO_MORE_AREA: std::sync::Mutex<Option<Rect>> = std::sync::Mutex::new(None);

pub(super) fn set_pinned_todo_more_area(area: Option<Rect>) {
    *PINNED_TODO_MORE_AREA
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = area;
}

#[cfg(test)]
pub(crate) fn set_pinned_todo_more_area_for_test(area: Option<Rect>) {
    set_pinned_todo_more_area(area);
}

pub(crate) fn pinned_todo_more_area() -> Option<Rect> {
    *PINNED_TODO_MORE_AREA
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
