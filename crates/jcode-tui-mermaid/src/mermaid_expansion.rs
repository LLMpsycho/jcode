use super::*;

/// Mermaid source keyed by the same content hash used by inline image markers.
/// This lets transcript clicks copy editable Mermaid text instead of PNG pixels.
static MERMAID_SOURCE_BY_HASH: LazyLock<Mutex<HashMap<u64, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static MERMAID_INLINE_EXPAND_LEVEL: LazyLock<Mutex<HashMap<u64, u8>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static MERMAID_INLINE_EXPAND_EPOCH: AtomicU64 = AtomicU64::new(0);
type MermaidInlineLevelGeometry = [(u16, u16); 3];

static MERMAID_INLINE_LEVEL_GEOMETRY: LazyLock<Mutex<HashMap<u64, MermaidInlineLevelGeometry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn mermaid_source_for_hash(hash: u64) -> Option<String> {
    MERMAID_SOURCE_BY_HASH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&hash)
        .cloned()
}

pub fn set_mermaid_inline_expand_level(hash: u64, level: u8) {
    if let Ok(mut levels) = MERMAID_INLINE_EXPAND_LEVEL.lock() {
        let previous = levels.get(&hash).copied().unwrap_or(0);
        let level = level.min(2);
        if level == 0 {
            levels.remove(&hash);
        } else {
            levels.insert(hash, level);
        }
        if previous != level {
            MERMAID_INLINE_EXPAND_EPOCH.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub fn mermaid_inline_expand_epoch() -> u64 {
    MERMAID_INLINE_EXPAND_EPOCH.load(Ordering::Relaxed)
}

pub fn next_distinct_mermaid_inline_level(hash: u64, current: u8) -> u8 {
    let Some(geometries) = MERMAID_INLINE_LEVEL_GEOMETRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&hash)
        .copied()
    else {
        return (current + 1) % 3;
    };
    let current = current.min(2);
    for offset in 1..=3 {
        let candidate = (current + offset) % 3;
        if geometries[candidate as usize] != geometries[current as usize] {
            return candidate;
        }
    }
    0
}

pub fn register_inline_level_geometries(hash: u64, geometries: [(u16, u16); 3]) {
    if let Ok(mut all) = MERMAID_INLINE_LEVEL_GEOMETRY.lock() {
        all.insert(hash, geometries);
    }
}

pub(crate) fn mermaid_inline_expand_level(hash: u64) -> u8 {
    MERMAID_INLINE_EXPAND_LEVEL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&hash)
        .copied()
        .unwrap_or(0)
}

pub(crate) fn remember_mermaid_source(hash: u64, content: &str) {
    if let Ok(mut sources) = MERMAID_SOURCE_BY_HASH.lock() {
        if sources.len() >= RENDER_CACHE_MAX && !sources.contains_key(&hash) {
            sources.clear();
        }
        sources.insert(hash, content.to_string());
    }
}

#[cfg(test)]
#[path = "mermaid_expansion_tests.rs"]
mod distinct_level_tests;
