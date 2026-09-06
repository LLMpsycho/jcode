#[test]
fn test_accumulate_streaming_output_tokens_uses_deltas() {
    let mut app = create_test_app();
    let mut seen = 0;

    app.streaming.streaming_tps_collect_output = true;
    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(10));

    app.accumulate_streaming_output_tokens(10, &mut seen);
    app.accumulate_streaming_output_tokens(30, &mut seen);
    app.accumulate_streaming_output_tokens(30, &mut seen);

    assert_eq!(app.streaming.streaming_total_output_tokens, 30);
    assert_eq!(app.streaming.streaming_tps_observed_output_tokens, 30);
    assert!(app.streaming.streaming_tps_observed_elapsed >= Duration::from_secs(9));
    assert_eq!(seen, 30);
}
#[test]
fn test_accumulate_streaming_output_tokens_ignores_hidden_output_phase() {
    let mut app = create_test_app();
    let mut seen = 0;

    app.accumulate_streaming_output_tokens(20, &mut seen);
    assert_eq!(app.streaming.streaming_total_output_tokens, 0);
    assert_eq!(app.streaming.streaming_tps_observed_output_tokens, 0);
    assert_eq!(seen, 20);

    app.streaming.streaming_tps_collect_output = true;
    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(10));
    app.accumulate_streaming_output_tokens(60, &mut seen);

    assert_eq!(app.streaming.streaming_total_output_tokens, 40);
    assert_eq!(app.streaming.streaming_tps_observed_output_tokens, 40);
    assert_eq!(seen, 60);
}
#[test]
fn test_compute_streaming_tps_uses_latest_observed_snapshot_instead_of_current_repaint_time() {
    let mut app = create_test_app();
    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(20));
    app.streaming.streaming_tps_observed_output_tokens = 40;
    app.streaming.streaming_tps_observed_elapsed = Duration::from_secs(10);

    let tps = app.compute_streaming_tps().expect("tps");
    assert!(tps > 3.9 && tps < 4.1, "unexpected tps: {tps}");
}
#[test]
fn test_compute_streaming_tps_does_not_decay_on_redundant_usage_snapshots() {
    let mut app = create_test_app();
    let mut seen = 0;

    app.streaming.streaming_tps_collect_output = true;
    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(10));
    app.accumulate_streaming_output_tokens(40, &mut seen);
    let initial_tps = app.compute_streaming_tps().expect("initial tps");

    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(30));
    app.accumulate_streaming_output_tokens(40, &mut seen);

    let tps = app.compute_streaming_tps().expect("tps");
    assert!(
        initial_tps > 3.9 && initial_tps < 4.1,
        "unexpected initial tps: {initial_tps}"
    );
    assert!(
        tps > 3.9 && tps < 4.1,
        "unexpected tps after redundant snapshot: {tps}"
    );
}
#[test]
fn test_compute_streaming_tps_bursty_stream_simulation_stays_constant_between_real_updates() {
    let mut app = create_test_app();
    let mut seen = 0;

    app.streaming.streaming_tps_collect_output = true;

    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(2));
    app.accumulate_streaming_output_tokens(10, &mut seen);
    let tps_after_first_burst = app.compute_streaming_tps().expect("tps after first burst");

    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(5));
    app.accumulate_streaming_output_tokens(10, &mut seen);
    let tps_after_idle_gap = app.compute_streaming_tps().expect("tps after idle gap");

    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(6));
    app.accumulate_streaming_output_tokens(30, &mut seen);
    let tps_after_second_burst = app.compute_streaming_tps().expect("tps after second burst");

    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(9));
    app.accumulate_streaming_output_tokens(30, &mut seen);
    let tps_after_second_idle_gap = app
        .compute_streaming_tps()
        .expect("tps after second idle gap");

    assert!(
        tps_after_first_burst > 4.9 && tps_after_first_burst < 5.1,
        "unexpected first burst tps: {tps_after_first_burst}"
    );
    assert!(
        (tps_after_idle_gap - tps_after_first_burst).abs() < 0.01,
        "tps changed without new tokens: first={tps_after_first_burst} idle={tps_after_idle_gap}"
    );
    assert!(
        tps_after_second_burst > 4.9 && tps_after_second_burst < 5.1,
        "unexpected second burst tps: {tps_after_second_burst}"
    );
    assert!(
        (tps_after_second_idle_gap - tps_after_second_burst).abs() < 0.01,
        "tps changed without new tokens: second={tps_after_second_burst} idle={tps_after_second_idle_gap}"
    );
}
#[test]
fn test_streaming_tps_timer_resume_pause_reset_lifecycle() {
    let mut app = create_test_app();

    assert_eq!(app.current_streaming_tps_elapsed(), Duration::ZERO);
    assert!(!app.streaming.streaming_tps_collect_output);

    app.resume_streaming_tps();
    assert!(app.streaming.streaming_tps_collect_output);
    assert!(app.streaming.streaming_tps_start.is_some());

    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(2));
    app.pause_streaming_tps(true);
    assert!(app.streaming.streaming_tps_collect_output);
    assert!(app.streaming.streaming_tps_start.is_none());
    assert!(app.streaming.streaming_tps_elapsed >= Duration::from_secs(2));

    let elapsed_after_pause = app.streaming.streaming_tps_elapsed;
    app.pause_streaming_tps(false);
    assert!(!app.streaming.streaming_tps_collect_output);
    assert_eq!(app.streaming.streaming_tps_elapsed, elapsed_after_pause);

    app.streaming.streaming_total_output_tokens = 42;
    app.streaming.streaming_tps_observed_output_tokens = 42;
    app.streaming.streaming_tps_observed_elapsed = elapsed_after_pause;
    app.reset_streaming_tps();

    assert_eq!(app.streaming.streaming_tps_elapsed, Duration::ZERO);
    assert_eq!(app.streaming.streaming_total_output_tokens, 0);
    assert_eq!(app.streaming.streaming_tps_observed_output_tokens, 0);
    assert_eq!(app.streaming.streaming_tps_observed_elapsed, Duration::ZERO);
    assert!(!app.streaming.streaming_tps_collect_output);
    assert!(app.streaming.streaming_tps_start.is_none());
}
#[test]
fn test_compute_streaming_tps_requires_tokens_and_minimum_elapsed() {
    let mut app = create_test_app();

    app.streaming.streaming_tps_observed_elapsed = Duration::from_secs(10);
    assert!(app.compute_streaming_tps().is_none());

    app.streaming.streaming_tps_observed_output_tokens = 10;
    app.streaming.streaming_tps_observed_elapsed = Duration::from_millis(100);
    assert!(app.compute_streaming_tps().is_none());

    app.streaming.streaming_tps_observed_elapsed = Duration::from_millis(250);
    let tps = app.compute_streaming_tps().expect("tps above threshold");
    assert!(tps > 35.0 && tps <= 40.0, "unexpected tps: {tps}");
}
#[test]
fn test_accumulate_streaming_output_tokens_counts_provider_usage_reset_once() {
    let mut app = create_test_app();
    let mut seen = 80;

    app.streaming.streaming_tps_collect_output = true;
    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(10));

    app.accumulate_streaming_output_tokens(20, &mut seen);
    assert_eq!(app.streaming.streaming_total_output_tokens, 20);
    assert_eq!(seen, 20);

    app.accumulate_streaming_output_tokens(25, &mut seen);
    assert_eq!(app.streaming.streaming_total_output_tokens, 25);
    assert_eq!(app.streaming.streaming_tps_observed_output_tokens, 25);
    assert_eq!(seen, 25);
}
#[test]
fn test_streaming_tps_late_final_usage_after_pause_uses_paused_elapsed() {
    let mut app = create_test_app();
    let mut seen = 0;

    app.streaming.streaming_tps_collect_output = true;
    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(10));
    app.pause_streaming_tps(true);

    assert!(app.streaming.streaming_tps_start.is_none());
    assert!(app.streaming.streaming_tps_elapsed >= Duration::from_secs(10));

    app.accumulate_streaming_output_tokens(40, &mut seen);

    assert_eq!(app.streaming.streaming_total_output_tokens, 40);
    assert_eq!(app.streaming.streaming_tps_observed_output_tokens, 40);
    assert!(app.streaming.streaming_tps_observed_elapsed >= Duration::from_secs(10));
    let tps = app.compute_streaming_tps().expect("late tps");
    assert!(tps > 3.0 && tps <= 4.0, "unexpected late tps: {tps}");
}
#[test]
fn test_begin_kv_cache_request_stops_tps_collection_until_output_resumes() {
    let mut app = create_test_app();
    let mut seen = 0;

    app.streaming.streaming_tps_collect_output = true;
    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(3));

    app.begin_kv_cache_request(&[Message::user("next")], &[], "system", "dynamic");

    assert!(!app.streaming.streaming_tps_collect_output);
    assert!(app.streaming.streaming_tps_start.is_none());
    assert!(app.streaming.streaming_tps_elapsed >= Duration::from_secs(3));

    app.accumulate_streaming_output_tokens(20, &mut seen);
    assert_eq!(app.streaming.streaming_total_output_tokens, 0);
    assert_eq!(seen, 20);

    app.resume_streaming_tps();
    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(2));
    app.accumulate_streaming_output_tokens(50, &mut seen);

    assert_eq!(app.streaming.streaming_total_output_tokens, 30);
    assert_eq!(app.streaming.streaming_tps_observed_output_tokens, 30);
    assert!(app.streaming.streaming_tps_observed_elapsed >= Duration::from_secs(5));
}
#[test]
fn test_streaming_tps_accumulates_multiple_generation_segments_excluding_paused_gap() {
    let mut app = create_test_app();
    let mut seen = 0;

    app.resume_streaming_tps();
    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(2));
    app.accumulate_streaming_output_tokens(10, &mut seen);

    app.pause_streaming_tps(true);
    let elapsed_after_first_segment = app.streaming.streaming_tps_elapsed;
    assert!(elapsed_after_first_segment >= Duration::from_secs(2));

    app.resume_streaming_tps();
    app.streaming.streaming_tps_start = Some(Instant::now() - Duration::from_secs(3));
    app.accumulate_streaming_output_tokens(30, &mut seen);

    assert_eq!(app.streaming.streaming_total_output_tokens, 30);
    assert_eq!(app.streaming.streaming_tps_observed_output_tokens, 30);
    assert!(app.streaming.streaming_tps_observed_elapsed >= Duration::from_secs(5));
    let tps = app.compute_streaming_tps().expect("segmented tps");
    assert!(tps > 5.0 && tps <= 6.0, "unexpected segmented tps: {tps}");
}
