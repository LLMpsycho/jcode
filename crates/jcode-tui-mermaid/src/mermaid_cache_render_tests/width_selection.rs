use super::*;

fn test_cache(name: &str) -> MermaidCache {
    let cache_dir = std::env::temp_dir().join(format!(
        "jcode-mermaid-width-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&cache_dir);
    fs::create_dir_all(&cache_dir).expect("create cache fixture");
    MermaidCache {
        entries: HashMap::new(),
        order: VecDeque::new(),
        cache_dir,
        width_miss_floor: HashMap::new(),
    }
}

#[test]
fn pane_expansion_prefers_wider_disk_rendition_then_memoizes_misses() {
    const HASH: u64 = 0xA11C_E55D_1A6A_0001;
    let mut cache = test_cache("prefer-wide");
    let narrow = CachedDiagram {
        path: cache.cache_dir.join(format!("{HASH:016x}_w100.png")),
        width: 100,
        height: 50,
    };
    cache.insert(
        HASH,
        RenderProfile {
            preferred_aspect_per_mille: Some(1000),
        },
        narrow,
    );
    let wide_path = cache.cache_dir.join(format!("{HASH:016x}_w220.png"));
    fs::write(&wide_path, []).expect("write wider cache fixture");

    let selected = cache
        .get_preferred_width_or_any_in_memory(HASH, Some(200))
        .expect("wider rendition");
    assert_eq!(selected.width, 220);

    fs::remove_file(&wide_path).expect("remove wider fixture");
    cache.entries.retain(|_, entry| entry.width == 100);
    cache.order.retain(|key| cache.entries.contains_key(key));
    let selected = cache
        .get_preferred_width_or_any_in_memory(HASH, Some(300))
        .expect("narrow fallback");
    assert_eq!(selected.width, 100);
    assert_eq!(cache.width_miss_floor.get(&HASH), Some(&300));
    let selected_again = cache
        .get_preferred_width_or_any_in_memory(HASH, Some(300))
        .expect("memoized narrow fallback");
    assert_eq!(selected_again.width, 100);
    assert_eq!(cache.width_miss_floor.get(&HASH), Some(&300));

    let _ = fs::remove_dir_all(&cache.cache_dir);
}
