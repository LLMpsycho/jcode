use super::*;

pub(super) fn render_context_compact(data: &InfoWidgetData, inner: Rect) -> Vec<Line<'static>> {
    if data.context_info_stale {
        return vec![Line::from(vec![
            Span::styled("Context ", Style::default().fg(rgb(140, 140, 150))),
            Span::styled("updating...", Style::default().fg(rgb(220, 180, 80))),
        ])];
    }
    let used_tokens = if let Some(observed) = data.observed_context_tokens {
        observed as usize
    } else if let Some(info) = &data.context_info {
        if info.total_chars == 0 {
            return Vec::new();
        }
        info.estimated_tokens()
    } else {
        return Vec::new();
    };
    let limit_tokens = data.context_limit.unwrap_or(DEFAULT_CONTEXT_LIMIT).max(1);
    let label = if data.is_compacting {
        "Context📦"
    } else {
        "Context"
    };

    vec![render_context_usage_line(
        label,
        used_tokens,
        limit_tokens,
        inner.width,
    )]
}
