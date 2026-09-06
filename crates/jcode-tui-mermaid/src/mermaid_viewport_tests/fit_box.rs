use super::*;

#[test]
fn expanded_fit_scales_up_to_fill_placeholder_rows() {
    let source = DynamicImage::new_rgba8(100, 100);
    let scaled = scale_to_fit_box(&source, 20, 20, (10, 10));

    assert!(matches!(scaled, Cow::Owned(_)));
    assert_eq!(cell_rect_for_image(&scaled, (10, 10)), (20, 20));
}
