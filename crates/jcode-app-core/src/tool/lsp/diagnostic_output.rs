use super::*;

pub(super) fn prioritized_diagnostics(items: &[Diagnostic]) -> Vec<&Diagnostic> {
    let mut ordered: Vec<_> = items.iter().collect();
    ordered.sort_by_key(|item| item.severity.map_or(1, |severity| severity.0));
    ordered
}

pub(super) fn diagnostic_evidence(items: &[Diagnostic]) -> Vec<Value> {
    prioritized_diagnostics(items)
        .into_iter()
        .take(32)
        .map(|item| {
            json!({
                "range": item.range,
                "severity": item.severity,
                "message": crate::message::redact_secrets(&item.message).chars().take(512).collect::<String>()
            })
        })
        .collect()
}

