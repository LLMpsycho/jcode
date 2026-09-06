//! Cluster geometry.

use super::*;

pub(super) fn stable_hash(values: &[String]) -> u64 {
    // Deterministic FNV-1a hash to keep auto-cluster IDs stable across runs.
    let mut hash: u64 = 0xcbf29ce484222325;
    for value in values {
        for byte in value.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

pub(super) fn average_embedding(graph: &MemoryGraph, member_ids: &[String]) -> Vec<f32> {
    let mut sum: Vec<f32> = Vec::new();
    let mut count = 0usize;

    for id in member_ids {
        let Some(emb) = graph.memories.get(id).and_then(|m| m.embedding.as_ref()) else {
            continue;
        };
        if sum.is_empty() {
            sum = vec![0.0; emb.len()];
        }
        if emb.len() != sum.len() {
            continue;
        }
        for (slot, value) in sum.iter_mut().zip(emb.iter()) {
            *slot += *value;
        }
        count += 1;
    }

    if count == 0 {
        return Vec::new();
    }

    let denom = count as f32;
    for value in &mut sum {
        *value /= denom;
    }
    sum
}

pub(super) fn infer_context_tag(
    manager: &MemoryManager,
    verified_ids: &[String],
    context_snippet: &str,
) -> Result<Option<(String, usize)>> {
    if verified_ids.len() < 2 {
        return Ok(None);
    }

    let project_graph = manager.load_project_graph()?;
    let global_graph = manager.load_global_graph()?;

    let mut tag_sets: Vec<HashSet<String>> = Vec::new();
    for id in verified_ids {
        let Some(memory) = project_graph
            .memories
            .get(id)
            .or_else(|| global_graph.memories.get(id))
        else {
            continue;
        };
        tag_sets.push(memory.tags.iter().map(|t| t.to_ascii_lowercase()).collect());
    }

    if tag_sets.len() < 2 {
        return Ok(None);
    }

    let mut common = tag_sets[0].clone();
    for tags in tag_sets.iter().skip(1) {
        common.retain(|tag| tags.contains(tag));
    }
    if !common.is_empty() {
        return Ok(None);
    }

    let Some(tag) = infer_candidate_tag(context_snippet) else {
        return Ok(None);
    };

    let mut applied = 0usize;
    for id in verified_ids {
        let already_tagged = project_graph
            .memories
            .get(id)
            .or_else(|| global_graph.memories.get(id))
            .map(|m| m.tags.iter().any(|t| t.eq_ignore_ascii_case(&tag)))
            .unwrap_or(false);
        if already_tagged {
            continue;
        }
        if manager.tag_memory(id, &tag).is_ok() {
            applied += 1;
        }
    }

    if applied > 0 {
        Ok(Some((tag, applied)))
    } else {
        Ok(None)
    }
}

pub(super) fn infer_candidate_tag(context: &str) -> Option<String> {
    const STOPWORDS: &[&str] = &[
        "about", "after", "again", "agent", "also", "because", "before", "being", "build", "check",
        "code", "context", "could", "debug", "extract", "from", "have", "into", "just", "memory",
        "might", "project", "really", "should", "that", "their", "there", "these", "they", "this",
        "those", "very", "what", "when", "with", "would", "your",
    ];

    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut token = String::new();
    let mut flush = |raw: &mut String| {
        if raw.is_empty() {
            return;
        }
        let candidate = raw.to_ascii_lowercase();
        raw.clear();
        if candidate.len() < 4 || candidate.len() > 32 {
            return;
        }
        if candidate.chars().all(|ch| ch.is_ascii_digit()) {
            return;
        }
        if STOPWORDS.contains(&candidate.as_str()) {
            return;
        }
        *counts.entry(candidate).or_insert(0) += 1;
    };

    for ch in context.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            token.push(ch);
        } else {
            flush(&mut token);
        }
    }
    flush(&mut token);

    counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .max_by_key(|(_, count)| *count)
        .map(|(tag, _)| tag)
}
