#[test]
#[ignore = "developer benchmark: profiles real /resume through first rendered picker frame"]
fn benchmark_real_resume_first_render_reports_timings() {
    invalidate_session_list_cache();

    let total_start = std::time::Instant::now();

    let loading_render_start = std::time::Instant::now();
    let mut loading_picker = SessionPicker::loading();
    let backend = ratatui::backend::TestBackend::new(120, 40);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| loading_picker.render(frame))
        .expect("render loading picker");
    let loading_render_elapsed = loading_render_start.elapsed();

    let load_start = std::time::Instant::now();
    let (server_groups, orphan_sessions) = load_sessions_grouped().expect("load sessions grouped");
    let load_elapsed = load_start.elapsed();
    let loaded_count: usize = server_groups
        .iter()
        .map(|group| group.sessions.len())
        .sum::<usize>()
        + orphan_sessions.len();

    let construct_start = std::time::Instant::now();
    let mut picker = SessionPicker::new_grouped(server_groups, orphan_sessions);
    let selected_before_render = picker.selected_session().map(|session| {
        (
            session.id.clone(),
            session.title.clone(),
            session.external_path.clone(),
            session.messages_preview.len(),
        )
    });
    let construct_elapsed = construct_start.elapsed();

    let first_render_start = std::time::Instant::now();
    terminal
        .draw(|frame| picker.render(frame))
        .expect("render loaded picker");
    let first_render_elapsed = first_render_start.elapsed();
    let selected_after_first_render = picker
        .selected_session()
        .map(|session| (session.id.clone(), session.messages_preview.len()));

    let second_render_start = std::time::Instant::now();
    terminal
        .draw(|frame| picker.render(frame))
        .expect("render loaded picker again");
    let second_render_elapsed = second_render_start.elapsed();

    eprintln!(
        "real resume first render: total={}ms loading_render={}ms load_grouped={}ms/{} construct={}ms first_render={}ms second_render={}ms selected_before={:?} selected_after={:?}",
        total_start.elapsed().as_millis(),
        loading_render_elapsed.as_millis(),
        load_elapsed.as_millis(),
        loaded_count,
        construct_elapsed.as_millis(),
        first_render_elapsed.as_millis(),
        second_render_elapsed.as_millis(),
        selected_before_render,
        selected_after_first_render,
    );
}
#[test]
#[ignore = "developer benchmark: profiles cached /resume first render latency"]
fn benchmark_real_resume_cached_first_render_reports_timings() {
    invalidate_session_list_cache();

    let refresh_start = std::time::Instant::now();
    let (_fresh_groups, _fresh_orphans) =
        load_sessions_grouped().expect("refresh sessions grouped");
    let refresh_elapsed = refresh_start.elapsed();

    let total_start = std::time::Instant::now();
    let cache_start = std::time::Instant::now();
    let (server_groups, orphan_sessions) =
        load_cached_sessions_grouped().expect("load cached sessions grouped");
    let cache_elapsed = cache_start.elapsed();
    let cached_count: usize = server_groups
        .iter()
        .map(|group| group.sessions.len())
        .sum::<usize>()
        + orphan_sessions.len();

    let construct_start = std::time::Instant::now();
    let mut picker = SessionPicker::new_grouped(server_groups, orphan_sessions);
    let construct_elapsed = construct_start.elapsed();

    let render_start = std::time::Instant::now();
    let backend = ratatui::backend::TestBackend::new(120, 40);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| picker.render(frame))
        .expect("render cached picker");
    let render_elapsed = render_start.elapsed();

    eprintln!(
        "real resume cached first render: total={}ms cache_read={}ms/{} construct={}ms first_render={}ms cache_refresh={}ms",
        total_start.elapsed().as_millis(),
        cache_elapsed.as_millis(),
        cached_count,
        construct_elapsed.as_millis(),
        render_elapsed.as_millis(),
        refresh_elapsed.as_millis(),
    );
}
#[test]
fn benchmark_resume_search_reports_incremental_timings() {
    let sessions = (0..500)
        .map(|idx| {
            let mut session = make_session(
                &format!("session_bench_{idx:03}"),
                &format!("bench-{idx:03}"),
                false,
                SessionStatus::Closed,
            );
            session.messages_preview = vec![PreviewMessage {
                role: "user".to_string(),
                content: format!("benchmark transcript content alpha beta zebra-token-{idx:03}"),
                tool_calls: Vec::new(),
                tool_data: None,
                timestamp: None,
            }];
            session.search_index = build_search_index(
                &session.id,
                &session.short_name,
                &session.title,
                session.working_dir.as_deref(),
                None,
                &session.messages_preview,
            );
            session
        })
        .collect::<Vec<_>>();

    let mut picker = SessionPicker::new(sessions);

    let first_start = std::time::Instant::now();
    picker.search_query = "z".to_string();
    picker.rebuild_items();
    let first_ms = first_start.elapsed().as_secs_f64() * 1000.0;

    let second_start = std::time::Instant::now();
    picker.search_query = "ze".to_string();
    picker.rebuild_items();
    let second_ms = second_start.elapsed().as_secs_f64() * 1000.0;

    let third_start = std::time::Instant::now();
    picker.search_query = "zebra-token-499".to_string();
    picker.rebuild_items();
    let third_ms = third_start.elapsed().as_secs_f64() * 1000.0;

    assert_eq!(picker.visible_sessions.len(), 1);
    eprintln!(
        "resume search bench: first_char={:.3}ms second_char={:.3}ms full_query={:.3}ms sessions=500",
        first_ms, second_ms, third_ms
    );
}
/// Profile the cost of a single preview-scroll frame. This is the operation the
/// user reported as slow: after scrolling, every frame rebuilds + re-wraps the
/// entire preview. We render once to warm any lazy state, then time repeated
/// scroll-and-render ticks and attribute time to the preview vs the (unchanged)
/// session list.
#[test]
#[ignore = "developer benchmark: profiles /resume preview scroll frame cost"]
fn benchmark_resume_op_preview_scroll_frame_cost() {
    const W: u16 = 120;
    const H: u16 = 40;
    let main_area = Rect::new(0, 0, W, H);
    // Mirrors render(): list = 40%, preview = 60% of the width.
    let list_area = Rect::new(0, 0, (W as f32 * 0.40) as u16, H);
    let preview_area = Rect::new(list_area.width, 0, W - list_area.width, H);

    for &(turns, paras) in &[(20usize, 2usize), (80, 3), (200, 4)] {
        let session = bench_large_session("scroll_bench", turns, paras);
        let preview_len = session.messages_preview.len();
        let mut picker = SessionPicker::new(vec![session]);
        picker.focus = PaneFocus::Preview;

        // Warm render (auto-scrolls to bottom, builds wrap state once).
        let _ = bench_render_full(&mut picker, W, H);

        const ITERS: usize = 60;
        let mut full_samples = Vec::with_capacity(ITERS);
        let mut preview_samples = Vec::with_capacity(ITERS);
        let mut list_samples = Vec::with_capacity(ITERS);
        for i in 0..ITERS {
            // Alternate scroll direction so we exercise both bounds.
            if i % 2 == 0 {
                picker.scroll_preview_up(1);
            } else {
                picker.scroll_preview_down(1);
            }
            full_samples.push(bench_render_full(&mut picker, W, H));
            preview_samples.push(bench_render_preview_only(&mut picker, preview_area));
            list_samples.push(bench_render_list_only(&mut picker, list_area));
        }

        let full = bench_median(full_samples);
        let preview = bench_median(preview_samples);
        let list = bench_median(list_samples);
        eprintln!(
            "preview scroll frame: turns={turns} paras={paras} preview_msgs={preview_len} \
             area={}x{} | full_frame={:>6.0}us preview_only={:>6.0}us list_only={:>6.0}us \
             (preview is {:.0}% of frame)",
            main_area.width,
            main_area.height,
            full.as_nanos() as f64 / 1000.0,
            preview.as_nanos() as f64 / 1000.0,
            list.as_nanos() as f64 / 1000.0,
            preview.as_nanos() as f64 / full.as_nanos().max(1) as f64 * 100.0,
        );
    }
}
/// Profile how list rendering scales with the number of sessions. Because
/// `render_session_list` rebuilds a `ListItem` for *every* session each frame
/// (not just the visible window), this should grow ~linearly with N even though
/// only ~H rows are visible. Relevant to scroll because the list is redrawn on
/// every preview-scroll frame too.
#[test]
#[ignore = "developer benchmark: profiles /resume session-list render scaling vs N"]
fn benchmark_resume_op_list_render_scaling() {
    const W: u16 = 48;
    const H: u16 = 40;
    let list_area = Rect::new(0, 0, W, H);

    for &n in &[50usize, 200, 1000, 3000] {
        let sessions: Vec<SessionInfo> = (0..n)
            .map(|i| {
                make_session(
                    &format!("scale_{i}"),
                    &format!("session {i}"),
                    false,
                    SessionStatus::Closed,
                )
            })
            .collect();
        let mut picker = SessionPicker::new(sessions);
        picker.focus = PaneFocus::Sessions;
        let _ = bench_render_list_only(&mut picker, list_area);

        const ITERS: usize = 30;
        let mut samples = Vec::with_capacity(ITERS);
        for _ in 0..ITERS {
            samples.push(bench_render_list_only(&mut picker, list_area));
        }
        let m = bench_median(samples);
        eprintln!(
            "list render scaling: N={n:>5} visible_rows~={} | list_render={:>7.0}us \
             ({:.1}us/session)",
            H.saturating_sub(2),
            m.as_nanos() as f64 / 1000.0,
            m.as_nanos() as f64 / 1000.0 / n as f64,
        );
    }
}
/// Profile a search keystroke (`rebuild_items` + the cached search narrowing)
/// as the query grows, plus the cost of clearing the query (the non-prefix /
/// backspace path that cannot reuse the narrowing cache).
#[test]
#[ignore = "developer benchmark: profiles /resume search keystroke cost"]
fn benchmark_resume_op_search_keystroke() {
    for &n in &[200usize, 1000, 3000] {
        let sessions: Vec<SessionInfo> = (0..n)
            .map(|i| {
                make_session(
                    &format!("search_{i}"),
                    &format!("session about topic {} number {i}", i % 17),
                    false,
                    SessionStatus::Closed,
                )
            })
            .collect();
        let mut picker = SessionPicker::new(sessions);

        // Progressive typing: each keystroke appends one char and rebuilds.
        let query = "session about topic 3";
        let mut typed = String::new();
        let mut keystroke_samples = Vec::new();
        for ch in query.chars() {
            typed.push(ch);
            picker.search_query = typed.clone();
            picker.search_active = true;
            let start = std::time::Instant::now();
            picker.rebuild_items();
            keystroke_samples.push(start.elapsed());
        }
        let typed_median = bench_median(keystroke_samples.clone());
        let typed_worst = keystroke_samples.iter().copied().max().unwrap();

        // Clearing the search (full re-scan, no narrowing cache reuse).
        picker.search_query.clear();
        picker.search_active = false;
        let clear_start = std::time::Instant::now();
        picker.rebuild_items();
        let clear_elapsed = clear_start.elapsed();

        eprintln!(
            "search keystroke: N={n:>5} | per_keystroke_median={:>6.0}us worst={:>6.0}us \
             clear_query={:>6.0}us",
            typed_median.as_nanos() as f64 / 1000.0,
            typed_worst.as_nanos() as f64 / 1000.0,
            clear_elapsed.as_nanos() as f64 / 1000.0,
        );
    }
}
/// Profile navigating the session list (next/previous) followed by a re-render,
/// across list sizes. Navigation resets preview scroll and triggers a full
/// re-render of both panes.
#[test]
#[ignore = "developer benchmark: profiles /resume list navigation frame cost"]
fn benchmark_resume_op_nav_frame_cost() {
    const W: u16 = 120;
    const H: u16 = 40;

    for &n in &[50usize, 500, 2000] {
        let sessions: Vec<SessionInfo> = (0..n)
            .map(|i| bench_large_session(&format!("nav_{i}"), 6, 2))
            .collect();
        let mut picker = SessionPicker::new(sessions);
        picker.focus = PaneFocus::Sessions;
        let _ = bench_render_full(&mut picker, W, H);

        const ITERS: usize = 40;
        let mut samples = Vec::with_capacity(ITERS);
        for i in 0..ITERS {
            if i % 2 == 0 {
                picker.next();
            } else {
                picker.previous();
            }
            samples.push(bench_render_full(&mut picker, W, H));
        }
        let m = bench_median(samples);
        eprintln!(
            "nav frame: N={n:>5} | nav+full_render_median={:>7.0}us",
            m.as_nanos() as f64 / 1000.0,
        );
    }
}
/// Profile constructing the picker (`new`) and the initial `rebuild_items`
/// across list sizes, isolating the non-IO construction cost that runs
/// synchronously when `/resume` opens.
#[test]
#[ignore = "developer benchmark: profiles /resume picker construction cost vs N"]
fn benchmark_resume_op_construction_cost() {
    for &n in &[200usize, 1000, 5000] {
        let sessions: Vec<SessionInfo> = (0..n)
            .map(|i| {
                make_session(
                    &format!("ctor_{i}"),
                    &format!("session {i}"),
                    false,
                    SessionStatus::Closed,
                )
            })
            .collect();

        const ITERS: usize = 20;
        let mut samples = Vec::with_capacity(ITERS);
        for _ in 0..ITERS {
            let clone = sessions.clone();
            let start = std::time::Instant::now();
            let _picker = SessionPicker::new(clone);
            samples.push(start.elapsed());
        }
        let m = bench_median(samples);
        eprintln!(
            "construction: N={n:>5} | new()+rebuild_items_median={:>7.0}us ({:.2}us/session)",
            m.as_nanos() as f64 / 1000.0,
            m.as_nanos() as f64 / 1000.0 / n as f64,
        );
    }
}
