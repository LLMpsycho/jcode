//! Confirming test for the "image renders above its tag line" report.
//!
//! Static analysis of the draw path (`ui_viewport.rs`) showed both image
//! branches keep Kitty placeholder cells strictly inside the image region,
//! which sits one wrapped line *below* the label/tag. The lowest-level
//! emitter is [`render_kitty_virtual_viewport`]: it is the only place that
//! writes the `U+10EEEE` placeholder char into buffer cells. If the in-buffer
//! geometry were the culprit, this emitter would have to paint a placeholder
//! on a row above its `area`. This test drives it directly across a sweep of
//! `scroll_y`/partial-visibility values and asserts that never happens, which
//! pins the real defect to terminal-side ghosting rather than buffer layout.

use super::*;

const PLACEHOLDER: char = '\u{10EEEE}';

#[test]
fn virtual_transmit_uses_compressed_png_payload() {
    let image = DynamicImage::ImageRgba8(image::ImageBuffer::from_pixel(
        256,
        256,
        image::Rgba([245, 245, 245, 255]),
    ));
    let transmit = kitty_transmit_virtual(&image, 7);
    let raw_base64_bytes = (256usize * 256 * 4).div_ceil(3) * 4;

    assert!(transmit.contains("f=100"));
    assert!(!transmit.contains("f=32"));
    assert!(
        transmit.len() < raw_base64_bytes / 10,
        "PNG transmit should be far smaller than raw RGBA: {} vs {} bytes",
        transmit.len(),
        raw_base64_bytes,
    );
}

#[test]
fn kitty_ids_are_process_unique_and_delete_payload_targets_one_image() {
    let first = kitty_viewport_unique_id(0x0000_0001_FFFF_FFFF);
    let second = kitty_viewport_unique_id(0xFFFF_FFFF_0000_0001);
    assert_ne!(
        first, second,
        "64-bit hash folds must never alias Kitty ids"
    );

    let delete = kitty_delete_image_payload(first);
    assert!(delete.contains("a=d,d=I"));
    assert!(delete.contains(&format!("i={first}")));
}

#[test]
fn kitty_cache_eviction_queues_terminal_image_deletion() {
    let _guard = crate::IMAGE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = take_kitty_delete_ids();
    let mut cache = KittyViewportCache::new();
    for index in 0..KITTY_VIEWPORT_STATE_MAX {
        cache.insert(
            index as u64,
            KittyViewportState {
                source_path: PathBuf::from(format!("/test/evict-{index}.png")),
                zoom_percent: 100,
                font_size: (8, 16),
                unique_id: 10_000 + index as u32,
                full_cols: 10,
                full_rows: 10,
                pending_transmit: None,
                pending_transmit_bytes: 0,
                fit_target: Some((10, 10)),
            },
        );
    }
    cache.get_mut(0).expect("touch oldest state");
    let index = KITTY_VIEWPORT_STATE_MAX;
    cache.insert(
        index as u64,
        KittyViewportState {
            source_path: PathBuf::from(format!("/test/evict-{index}.png")),
            zoom_percent: 100,
            font_size: (8, 16),
            unique_id: 10_000 + index as u32,
            full_cols: 10,
            full_rows: 10,
            pending_transmit: None,
            pending_transmit_bytes: 0,
            fit_target: Some((10, 10)),
        },
    );
    assert_eq!(cache.entries.len(), KITTY_VIEWPORT_STATE_MAX);
    assert!(
        cache.entries.contains_key(&0),
        "recently touched state stays hot"
    );
    assert!(
        !cache.entries.contains_key(&1),
        "least-recent state is evicted"
    );
    assert_eq!(take_kitty_delete_ids(), vec![10_001]);
}

#[test]
fn kitty_cache_bounds_not_yet_drawn_transmissions_by_bytes() {
    let _guard = crate::IMAGE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = take_kitty_delete_ids();
    let mut cache = KittyViewportCache::new();
    let accounted = KITTY_VIEWPORT_PENDING_MAX_BYTES / 2 + 1;
    for index in 0..3u64 {
        cache.insert(
            index,
            KittyViewportState {
                source_path: PathBuf::from(format!("/test/pending-{index}.png")),
                zoom_percent: 100,
                font_size: (8, 16),
                unique_id: 20_000 + index as u32,
                full_cols: 10,
                full_rows: 10,
                pending_transmit: Some(String::from("synthetic")),
                pending_transmit_bytes: accounted,
                fit_target: Some((10, 10)),
            },
        );
    }
    assert_eq!(
        cache.entries.len(),
        1,
        "byte budget should evict old prewarms"
    );
    assert_eq!(cache.total_pending_transmit_bytes, accounted);
    assert!(
        take_kitty_delete_ids().is_empty(),
        "never-drawn prewarms have no terminal allocation to delete"
    );

    let (_, pending) = cache.take_pending_transmit(2).expect("newest state");
    assert_eq!(pending.as_deref(), Some("synthetic"));
    assert_eq!(cache.total_pending_transmit_bytes, 0);
}

#[test]
fn pending_terminal_cleanup_renders_without_changing_visible_cell_text() {
    let _guard = crate::IMAGE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = take_kitty_delete_ids();
    queue_kitty_delete(42_424);
    let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
    buf.cell_mut((0, 0)).expect("fixture cell").set_symbol("X");

    assert!(crate::render_pending_terminal_image_cleanup(&mut buf));
    let symbol = buf.cell((0, 0)).expect("cleanup cell").symbol();
    assert!(symbol.contains("a=d,d=I,i=42424"));
    assert!(symbol.ends_with('X'), "visible symbol must be preserved");
    assert!(take_kitty_delete_ids().is_empty());
}

#[test]
fn unchanged_png_transmit_reuses_exact_file_bytes() {
    use image::ImageEncoder as _;

    let pixels = image::RgbaImage::from_pixel(7, 5, image::Rgba([9, 71, 203, 255]));
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(pixels.as_raw(), 7, 5, image::ExtendedColorType::Rgba8)
        .expect("encode fixture");
    let path = std::env::temp_dir().join(format!(
        "jcode-kitty-source-{}-{}.png",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(&path, &png).expect("write fixture");

    let actual = kitty_transmit_png_path(&path, 19).expect("direct PNG transmit");
    let expected = kitty_transmit_payload(&png, 19, "f=100");
    assert_eq!(actual, expected, "source PNG bytes must not be re-encoded");

    let _ = fs::remove_file(path);
}

#[test]
fn halfblocks_partial_scroll_reuses_fitted_source_and_identical_slice() {
    let _guard = crate::IMAGE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    const HASH: u64 = 0x4841_4C46_424C_4B53;
    let path = PathBuf::from("/test/halfblocks-scroll.png");
    let source = DynamicImage::ImageRgba8(image::ImageBuffer::from_pixel(
        160,
        128,
        image::Rgba([31, 127, 223, 255]),
    ));
    SOURCE_CACHE
        .lock()
        .unwrap()
        .insert(HASH, path.clone(), source);
    FITTED_SOURCE_CACHE.lock().unwrap().remove_hash(HASH);
    IMAGE_STATE.lock().unwrap().remove(&HASH);

    #[allow(deprecated)]
    let mut picker = Picker::from_fontsize((2, 4));
    picker.set_protocol_type(ProtocolType::Halfblocks);
    let cached = CachedDiagram {
        path: path.clone(),
        width: 160,
        height: 128,
    };
    let area = Rect::new(0, 0, 42, 5);
    let mut buf = Buffer::empty(Rect::new(0, 0, 42, 16));
    let stats_before = debug_stats();

    assert!(render_non_kitty_fit_stable(
        HASH,
        area,
        &mut buf,
        42,
        16,
        4,
        false,
        true,
        &picker,
        cached.clone(),
    ));
    let first_stats = debug_stats();
    assert_eq!(
        first_stats
            .fit_protocol_rebuilds
            .saturating_sub(stats_before.fit_protocol_rebuilds),
        1,
        "the first visible slice should build one protocol"
    );
    {
        let state = IMAGE_STATE.lock().unwrap();
        let state = state.get(&HASH).expect("halfblocks viewport state");
        assert_eq!(state.resize_mode, ResizeMode::FitViewport);
        assert_eq!(
            state.last_viewport,
            Some(ViewportState {
                scroll_x_px: 0,
                scroll_y_px: 16,
                view_w_px: 80,
                view_h_px: 20,
            })
        );
    }
    assert!(
        SOURCE_CACHE.lock().unwrap().entries.get(&HASH).is_none(),
        "full decoded original should be released after fitting"
    );
    assert_eq!(
        FITTED_SOURCE_CACHE
            .lock()
            .unwrap()
            .entries
            .keys()
            .filter(|key| key.hash == HASH)
            .count(),
        1
    );

    // Rendering the identical scroll slice must reuse protocol state rather
    // than recropping/re-encoding it.
    assert!(render_non_kitty_fit_stable(
        HASH, area, &mut buf, 42, 16, 4, false, true, &picker, cached,
    ));
    let second_stats = debug_stats();
    assert_eq!(
        second_stats
            .fit_protocol_rebuilds
            .saturating_sub(first_stats.fit_protocol_rebuilds),
        0
    );
    assert_eq!(
        second_stats
            .fit_state_reuse_hits
            .saturating_sub(first_stats.fit_state_reuse_hits),
        1
    );

    IMAGE_STATE.lock().unwrap().remove(&HASH);
    SOURCE_CACHE.lock().unwrap().remove(HASH);
    FITTED_SOURCE_CACHE.lock().unwrap().remove_hash(HASH);
}

/// Seed `KITTY_VIEWPORT_STATE` with a fit entry so the emitter has an id to
/// address without needing a real terminal/transmit.
fn seed_state(hash: u64, full_cols: u16, full_rows: u16) {
    let mut cache = KITTY_VIEWPORT_STATE.lock().unwrap();
    cache.insert(
        hash,
        KittyViewportState {
            source_path: std::path::PathBuf::from("/test/leak.png"),
            zoom_percent: 100,
            font_size: (8, 16),
            unique_id: 0x00AABBCC,
            full_cols,
            full_rows,
            pending_transmit: Some(String::from("\x1b_Gtransmit\x1b\\")),
            pending_transmit_bytes: "\x1b_Gtransmit\x1b\\".len(),
            fit_target: Some((full_cols, full_rows)),
        },
    );
}

/// True if any cell at row `y` carries a Kitty placeholder char.
fn row_has_placeholder(buf: &Buffer, y: u16) -> bool {
    let area = *buf.area();
    (area.left()..area.right()).any(|x| {
        buf.cell((x, y))
            .is_some_and(|c| c.symbol().contains(PLACEHOLDER))
    })
}

/// The emitter must never paint placeholders above its own `area`, for any
/// scroll position or partial-visibility height. The buffer has sentinel
/// rows above the image area that stand in for the label/tag line.
#[test]
fn placeholders_never_leak_above_image_area() {
    let _guard = crate::IMAGE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let hash = 0xDEAD_BEEF_u64;
    let full_cols = 20;
    let full_rows = 30;
    seed_state(hash, full_cols, full_rows);

    // Buffer taller/wider than the image area, with several "tag" rows on top.
    let buf_w = full_cols + 4;
    let buf_h = full_rows + 8;
    let label_rows = 3u16; // rows 0..3 stand in for blank + label + spacer

    // Sweep: how many of the image's top rows are scrolled off (skip_rows),
    // which also drives the visible height that the draw path would request.
    for skip_rows in 0..full_rows {
        // Re-seed each iteration: render consumes pending_transmit.
        seed_state(hash, full_cols, full_rows);
        let visible_height = full_rows - skip_rows;

        let mut buf = Buffer::empty(Rect::new(0, 0, buf_w, buf_h));
        // Mark the label/tag region with a sentinel so any overwrite is loud.
        for y in 0..label_rows {
            for x in 0..buf_w {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol("T");
                }
            }
        }

        let image_area = Rect {
            x: 1,
            y: label_rows,
            width: full_cols,
            height: visible_height,
        };

        let ok = render_kitty_virtual_viewport(
            hash,
            image_area,
            &mut buf,
            0,
            skip_rows,
            full_cols.min(image_area.width),
            visible_height,
        );
        assert!(ok, "viewport render failed for skip_rows={skip_rows}");

        // No placeholder may sit on any row above the image area.
        for y in 0..label_rows {
            assert!(
                !row_has_placeholder(&buf, y),
                "placeholder leaked onto tag row {y} (skip_rows={skip_rows})"
            );
        }
        // The label/tag sentinel cells must be untouched.
        for y in 0..label_rows {
            for x in 0..buf_w {
                let sym = buf.cell((x, y)).map(|c| c.symbol().to_string());
                assert_eq!(
                    sym.as_deref(),
                    Some("T"),
                    "tag cell ({x},{y}) overwritten (skip_rows={skip_rows})"
                );
            }
        }
        // Sanity: the first image row should actually carry a placeholder, so
        // the test is exercising the real emission and not a no-op.
        assert!(
            row_has_placeholder(&buf, image_area.y),
            "expected placeholders on first image row (skip_rows={skip_rows})"
        );
    }
}
