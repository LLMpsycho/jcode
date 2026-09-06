use super::*;

fn sample(timestamp_ms: u64, render_ms: f64, changed: Option<usize>) -> DrawCallAttribution {
    DrawCallAttribution {
        timestamp_ms,
        total_ms: render_ms + 1.0,
        render_ms,
        backend_flush_ms: 1.0,
        changed_cells: changed,
        total_cells: Some(1000),
        force_full_redraw: false,
        input: FrameInputAttribution::default(),
    }
}

fn clear_draw_call_history() {
    draw_call_history()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

#[test]
fn draw_call_history_records_all_draws_and_summarizes() {
    // Single test covers both summary math and the ring-buffer bound so we
    // never race two tests on the shared static history.
    clear_draw_call_history();
    record_draw_call_attribution(sample(1_000, 2.0, Some(50)));
    record_draw_call_attribution(sample(1_033, 4.0, Some(150)));
    record_draw_call_attribution(sample(1_066, 6.0, Some(250)));

    let payload = debug_draw_call_history(8);
    assert_eq!(payload["buffered_samples"], 3);
    assert_eq!(payload["window_ms"], 66);
    // (3 - 1) draws / 66ms window ~= 30.3 draws/sec
    let dps = payload["summary"]["draws_per_second"].as_f64().unwrap();
    assert!((dps - 30.30).abs() < 0.5, "draws_per_second = {dps}");
    let avg_render = payload["summary"]["render_ms"]["avg"].as_f64().unwrap();
    assert!((avg_render - 4.0).abs() < 1e-9);
    // (50 + 150 + 250) / 3 / 1000 = 0.15
    let ratio = payload["summary"]["avg_changed_cell_ratio"]
        .as_f64()
        .unwrap();
    assert!((ratio - 0.15).abs() < 1e-9, "ratio = {ratio}");
    assert_eq!(payload["samples"].as_array().unwrap().len(), 3);

    // The ring buffer stays bounded.
    clear_draw_call_history();
    for i in 0..(DRAW_CALL_HISTORY_MAX_SAMPLES + 10) {
        record_draw_call_attribution(sample(i as u64, 1.0, None));
    }
    let payload = debug_draw_call_history(DRAW_CALL_HISTORY_MAX_SAMPLES);
    assert_eq!(payload["buffered_samples"], DRAW_CALL_HISTORY_MAX_SAMPLES);
    clear_draw_call_history();
}

#[test]
fn stability_hash_span_iteration_matches_plain_text_hash() {
    use ratatui::text::{Line, Span};
    // The span-iteration hash must equal what hashing the concatenated
    // plain text produced before the optimization, so historical hash
    // comparisons (flicker detection) stay stable across span splits.
    let split = vec![
        Line::from(vec![Span::raw("hello "), Span::raw("world")]),
        Line::from("second line"),
    ];
    let merged = vec![Line::from("hello world"), Line::from("second line")];
    let a = viewport_stability_hash(&split, &[1], 80, 2);
    let b = viewport_stability_hash(&merged, &[1], 80, 2);
    assert_eq!(a, b);

    // Differing content must still differ.
    let other = vec![Line::from("hello world!"), Line::from("second line")];
    let c = viewport_stability_hash(&other, &[1], 80, 2);
    assert_ne!(a, c);
}
