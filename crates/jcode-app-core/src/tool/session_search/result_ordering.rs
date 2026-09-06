//! Result ordering.

use super::*;

pub(super) fn compare_results(a: &SearchResult, b: &SearchResult) -> std::cmp::Ordering {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| b.updated_at.cmp(&a.updated_at))
        .then_with(|| a.session_id.cmp(&b.session_id))
        .then_with(|| a.message_index.cmp(&b.message_index))
}

pub(super) fn group_and_limit_results(
    results: Vec<SearchResult>,
    options: &SearchOptions,
) -> Vec<SearchResult> {
    let mut grouped = Vec::new();
    let mut per_session: HashMap<String, usize> = HashMap::new();

    for result in results {
        let count = per_session.entry(result.session_id.clone()).or_default();
        if *count >= options.max_per_session {
            continue;
        }
        *count += 1;
        grouped.push(result);
        if grouped.len() >= options.limit {
            break;
        }
    }

    grouped
}
