use super::*;

pub(super) fn compact_run_id(run_id: &str) -> String {
    if run_id.width() <= 22 {
        run_id.to_string()
    } else {
        let prefix: String = run_id.chars().take(18).collect();
        format!("{}…", prefix)
    }
}

pub(super) fn render_overnight_progress_line(
    card: &crate::overnight::OvernightProgressCard,
    inner_width: usize,
    filled_style: Style,
    empty_style: Style,
    label_style: Style,
    text_style: Style,
) -> Line<'static> {
    let percent = card.progress_percent.clamp(0.0, 100.0);
    let label = format!("{:>3}%", percent.round() as u32);
    let summary = format!("{} / {}", card.elapsed_label, card.target_duration_label);
    let separator = " · ";
    let fixed_width = 1 + label.width() + separator.width();
    let bar_width = if inner_width >= 56 {
        18
    } else if inner_width >= 40 {
        14
    } else if inner_width >= 28 {
        10
    } else {
        6
    }
    .min(inner_width.saturating_sub(fixed_width).max(1));
    let filled = ((percent / 100.0) * bar_width as f32).round() as usize;
    let filled = filled.min(bar_width);
    let empty = bar_width.saturating_sub(filled);
    let line = Line::from(vec![
        Span::styled("█".repeat(filled), filled_style),
        Span::styled("░".repeat(empty), empty_style),
        Span::styled(" ", label_style),
        Span::styled(label, label_style),
        Span::styled(separator, label_style),
        Span::styled(summary, text_style),
    ]);
    super::super::truncate_line_with_ellipsis_to_width(&line, inner_width)
}

pub(super) fn push_wrapped_kv_line(
    content: &mut Vec<Line<'static>>,
    label: &str,
    value: &str,
    inner_width: usize,
    label_style: Style,
    value_style: Style,
) {
    let prefix = format!("{}: ", label);
    let prefix_width = prefix.width();
    let available = inner_width.saturating_sub(prefix_width).max(1);
    let chunks = split_by_display_width(value.trim(), available);
    if chunks.is_empty() {
        return;
    }
    for (idx, chunk) in chunks.into_iter().enumerate() {
        if idx == 0 {
            content.push(super::super::truncate_line_with_ellipsis_to_width(
                &Line::from(vec![
                    Span::styled(prefix.clone(), label_style),
                    Span::styled(chunk, value_style),
                ]),
                inner_width,
            ));
        } else {
            content.push(super::super::truncate_line_with_ellipsis_to_width(
                &Line::from(vec![
                    Span::styled(" ".repeat(prefix_width), label_style),
                    Span::styled(chunk, value_style),
                ]),
                inner_width,
            ));
        }
    }
}

pub(super) fn format_overnight_task_counts(
    card: &crate::overnight::OvernightProgressCard,
) -> String {
    let counts = &card.task_summary.counts;
    format!(
        "{} complete, {} active, {} blocked, {} deferred · {} total, {} validated",
        counts.completed,
        counts.active,
        counts.blocked,
        counts.deferred,
        card.task_summary.total,
        card.task_summary.validated
    )
}
