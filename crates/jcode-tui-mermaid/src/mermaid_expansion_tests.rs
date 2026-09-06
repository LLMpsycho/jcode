use super::*;

#[test]
fn skips_duplicate_levels_in_two_size_cycle() {
    let hash = 0xd157_1ac7;
    register_inline_level_geometries(hash, [(10, 20), (20, 40), (20, 40)]);
    assert_eq!(next_distinct_mermaid_inline_level(hash, 0), 1);
    assert_eq!(next_distinct_mermaid_inline_level(hash, 1), 0);
}

#[test]
fn preserves_three_distinct_levels_and_collapses_one_size_cycle() {
    let three = 0xd157_1ac8;
    register_inline_level_geometries(three, [(10, 20), (20, 40), (30, 60)]);
    assert_eq!(next_distinct_mermaid_inline_level(three, 0), 1);
    assert_eq!(next_distinct_mermaid_inline_level(three, 1), 2);
    assert_eq!(next_distinct_mermaid_inline_level(three, 2), 0);

    let one = 0xd157_1ac9;
    register_inline_level_geometries(one, [(10, 20); 3]);
    assert_eq!(next_distinct_mermaid_inline_level(one, 0), 0);
}
