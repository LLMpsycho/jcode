/// The lazy prewarm must fire exactly on first mermaid detection, so a
/// diagram-free session never loads the font DB and a diagram session
/// warms it before the render path needs it.
#[test]
fn mermaid_detection_triggers_font_db_prewarm() {
    assert!(!super::is_mermaid_lang("rust"), "sanity: non-mermaid");
    // No spawn yet for non-mermaid langs (flag may already be set if
    // another test rendered a diagram first, so only assert the positive
    // path below).
    assert!(super::is_mermaid_lang("mermaid"));
    assert!(
        crate::SVG_FONT_DB_PREWARM_STARTED.get().is_some(),
        "first mermaid sighting must kick off the font-DB prewarm"
    );
}
