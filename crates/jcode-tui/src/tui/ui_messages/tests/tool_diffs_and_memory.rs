#[test]
fn render_tool_message_colors_high_token_badge() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "x".repeat(48_000),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_3".to_string(),
            name: "read".to_string(),
            input: serde_json::json!({"file_path": "src/main.rs"}),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off);
    let badge_span = lines[0]
        .spans
        .iter()
        .find(|span| span.content.contains("12k tok"))
        .expect("missing token badge");

    assert_eq!(badge_span.style.fg, Some(rgb(224, 118, 118)));
}
#[test]
fn render_tool_message_shows_inline_diff_for_pascal_case_multiedit() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "Edited demo.txt\n\nApplied:\n  ✓ Edit 1: replaced 1 occurrence\n\nTotal: 1 applied, 0 failed\n"
            .to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("demo.txt".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_multiedit_pascal".to_string(),
            name: "MultiEdit".to_string(),
            input: serde_json::json!({
                "file_path": "demo.txt",
                "edits": [
                    {"old_string": "old line\n", "new_string": "new line\n"}
                ]
            }),
            intent: None, thought_signature: None, }),
    };

    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Inline);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("┌─ diff"), "plain={plain}");
    assert!(plain.contains("old line"), "plain={plain}");
    assert!(plain.contains("new line"), "plain={plain}");
}
#[test]
fn render_tool_message_labels_single_file_apply_patch_diff() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "✓ src/example.rs: modified (1 hunks)".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("src/example.rs".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_apply_patch_single".to_string(),
            name: "apply_patch".to_string(),
            input: serde_json::json!({
                "intent": "Update example behavior",
                "patch_text": "*** Begin Patch\n*** Update File: src/example.rs\n@@\n-old_value\n+new_value\n*** End Patch\n"
            }),
            intent: Some("Update example behavior".to_string()),
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Inline);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("┌─ diff · src/example.rs"), "plain={plain}");
    assert!(plain.contains("old_value"), "plain={plain}");
    assert!(plain.contains("new_value"), "plain={plain}");
}
#[test]
fn render_tool_message_preserves_multi_file_apply_patch_boundaries() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "✓ a.txt: modified (1 hunks)\n1- old a\n1+ new a\n✓ b.txt: modified (1 hunks)\n1- old b\n1+ new b\n".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("2 files".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_apply_patch_multi".to_string(),
            name: "apply_patch".to_string(),
            input: serde_json::json!({
                "intent": "Update both examples",
                "patch_text": "*** Begin Patch\n*** Update File: a.txt\n@@\n-old a\n+new a\n*** Update File: b.txt\n@@\n-old b\n+new b\n*** End Patch\n"
            }),
            intent: Some("Update both examples".to_string()),
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Inline);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    let a_header = plain.find("┌─ diff · a.txt").expect("missing a.txt header");
    let old_a = plain.find("old a").expect("missing a.txt deletion");
    let new_a = plain.find("new a").expect("missing a.txt addition");
    let b_header = plain
        .find("├─ diff · b.txt")
        .expect("missing b.txt boundary");
    let old_b = plain.find("old b").expect("missing b.txt deletion");
    let new_b = plain.find("new b").expect("missing b.txt addition");
    assert!(
        a_header < old_a && old_a < new_a && new_a < b_header,
        "plain={plain}"
    );
    assert!(b_header < old_b && old_b < new_b, "plain={plain}");
}
#[test]
fn render_tool_message_shows_numbered_write_result_diff_after_input_compaction() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "Created /tmp/head-to-head.html (2 lines):\n1+ <!doctype html>\n2+ <html lang=\"en\">\n..."
            .to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("/tmp/head-to-head.html".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_write_compacted".to_string(),
            name: "write".to_string(),
            input: serde_json::json!({"file_path": "/tmp/head-to-head.html"}),
            intent: Some("Create an honest data-driven benchmark comparison page".to_string()),
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Inline);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        lines[0].spans.iter().any(|span| span.content == "+2"),
        "plain={plain}"
    );
    assert!(plain.contains("┌─ diff"), "plain={plain}");
    assert!(plain.contains("<!doctype html>"), "plain={plain}");
    assert!(plain.contains("<html lang=\"en\">"), "plain={plain}");
}
#[test]
fn render_tool_message_never_draws_an_empty_edit_diff_frame() {
    for (name, content) in [
        ("write", "Created empty.txt (0 lines):\n"),
        ("edit", "Edited demo.txt: replaced 1 occurrence(s)"),
        (
            "multiedit",
            "Edited demo.txt\n\nTotal: 1 applied, 0 failed\n",
        ),
        ("patch", "Patch applied successfully"),
        ("apply_patch", "✓ demo.txt: modified (1 hunks)"),
    ] {
        let msg = DisplayMessage {
            role: "tool".to_string(),
            content: content.to_string(),
            tool_calls: Vec::new(),
            duration_secs: None,
            title: Some("demo.txt".to_string()),
            tool_data: Some(crate::message::ToolCall {
                id: format!("call_{name}_compacted"),
                name: name.to_string(),
                input: serde_json::json!({"file_path": "demo.txt"}),
                intent: None,
                thought_signature: None,
            }),
        };

        let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Inline);
        let plain = lines
            .iter()
            .map(extract_line_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!plain.contains("┌─ diff"), "tool={name}, plain={plain}");
        assert!(!plain.contains("(+0 -0)"), "tool={name}, plain={plain}");
    }
}
#[test]
fn render_tool_message_marks_failed_apply_patch_without_empty_diff() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content:
            "[apply_patch] ✗ /tmp/main.rs: Failed to find expected lines in /tmp/main.rs:\nfn missing() {}"
                .to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("/tmp/main.rs".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_apply_patch_failed".to_string(),
            name: "apply_patch".to_string(),
            input: serde_json::json!({"file_path": "/tmp/main.rs"}),
            intent: Some("Replace the benchmark placeholder".to_string()),
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Inline);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        plain.trim_start().starts_with("✗ apply_patch"),
        "plain={plain}"
    );
    assert!(!plain.contains("┌─ diff"), "plain={plain}");
    assert!(!plain.contains("(+0 -0)"), "plain={plain}");
}
#[test]
fn render_tool_message_inline_mode_truncates_large_diffs() {
    let old = (1..=7)
        .map(|i| format!("old line {i}\n"))
        .collect::<String>();
    let new = (1..=7)
        .map(|i| format!("new line {i} suffix_{i}_abcdefghijklmnopqrstuvwxyz0123456789\n"))
        .collect::<String>();
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "Edited demo.txt".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("demo.txt".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_edit_inline_truncated".to_string(),
            name: "edit".to_string(),
            input: serde_json::json!({
                "file_path": "demo.txt",
                "old_string": old,
                "new_string": new,
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 40, crate::config::DiffDisplayMode::Inline);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("... 2 more changes ..."), "plain={plain}");
    assert!(plain.contains("old line 3"), "plain={plain}");
    assert!(!plain.contains("old line 7"), "plain={plain}");
    assert!(
        !plain.contains("new line 1 suffix_1_abcdefghijklmnopqrstuvwxyz0123456789"),
        "plain={plain}"
    );
    assert!(plain.contains("suffix_2_abcdefghijklm…"), "plain={plain}");
}
#[test]
fn render_tool_message_full_inline_mode_shows_full_diff() {
    let old = (1..=7)
        .map(|i| format!("old line {i}\n"))
        .collect::<String>();
    let new = (1..=7)
        .map(|i| format!("new line {i} suffix_{i}_abcdefghijklmnopqrstuvwxyz0123456789\n"))
        .collect::<String>();
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "Edited demo.txt".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("demo.txt".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_edit_inline_full".to_string(),
            name: "edit".to_string(),
            input: serde_json::json!({
                "file_path": "demo.txt",
                "old_string": old,
                "new_string": new,
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 40, crate::config::DiffDisplayMode::FullInline);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(!plain.contains("more changes"), "plain={plain}");
    assert!(plain.contains("old line 4"), "plain={plain}");
    assert!(
        plain.contains("new line 4 suffix_4_abcdefghijklmnopqrstuvwxyz0123456789"),
        "plain={plain}"
    );
    assert!(!plain.contains('…'), "plain={plain}");
}
#[test]
fn render_tool_message_memory_recall_centered_mode_left_aligns_with_padding() {
    let saved = crate::tui::markdown::center_code_blocks();
    crate::tui::markdown::set_center_code_blocks(true);
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: concat!(
            "- [fact] Centered mode should keep the recall card centered\n",
            "- [preference] The user likes visible side gutters"
        )
        .to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_memory_recall_centered".to_string(),
            name: "memory".to_string(),
            input: serde_json::json!({
                "action": "recall",
                "query": "centered mode"
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off);
    let rendered: Vec<String> = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect();

    assert!(!rendered.is_empty(), "expected rendered recall card");
    assert!(
        rendered.iter().all(|line| line.starts_with("  ")),
        "centered recall card should include shared left padding: {rendered:?}"
    );
    assert_eq!(
        lines[0].alignment,
        Some(ratatui::layout::Alignment::Left),
        "centered recall card header should be left-aligned after padding"
    );
    assert!(
        rendered[0]
            .trim_start()
            .starts_with("🧠 recalled 2 memories"),
        "unexpected recall header: {rendered:?}"
    );

    crate::tui::markdown::set_center_code_blocks(saved);
}
#[test]
fn render_tool_message_memory_store_centered_mode_left_aligns_with_padding() {
    let saved = crate::tui::markdown::center_code_blocks();
    crate::tui::markdown::set_center_code_blocks(true);
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "Saved memory".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_memory_store_centered".to_string(),
            name: "memory".to_string(),
            input: serde_json::json!({
                "action": "remember",
                "category": "fact",
                "content": "Centered mode should pad saved memory cards too"
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off);
    let rendered: Vec<String> = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect();

    assert!(!rendered.is_empty(), "expected rendered saved-memory card");
    assert!(
        rendered.iter().all(|line| line.starts_with("  ")),
        "centered saved-memory card should include shared left padding: {rendered:?}"
    );
    assert_eq!(
        lines[0].alignment,
        Some(ratatui::layout::Alignment::Left),
        "centered saved-memory card should be left-aligned after padding"
    );

    crate::tui::markdown::set_center_code_blocks(saved);
}
#[test]
fn render_tool_message_shows_swarm_spawn_prompt_summary() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "spawned".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_swarm_spawn".to_string(),
            name: "swarm".to_string(),
            input: serde_json::json!({
                "action": "spawn",
                "prompt": "Extract the restart command cluster from cli commands and validate it"
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off);
    let rendered: String = lines[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

    assert!(rendered.contains("swarm spawn"), "rendered={rendered}");
    assert!(
        rendered.contains("Extract the restart command cluster"),
        "rendered={rendered}"
    );
}
#[test]
fn render_tool_message_batch_subcall_shows_swarm_dm_details() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "--- [1] swarm ---\nDone\n\nCompleted: 1 succeeded, 0 failed".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_batch_swarm".to_string(),
            name: "batch".to_string(),
            input: serde_json::json!({
                "tool_calls": [
                    {
                        "tool": "swarm",
                        "action": "dm",
                        "to_session": "shark",
                        "message": "Please validate the restart extraction and report back"
                    }
                ]
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off);
    let rendered = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("swarm dm → shark"), "rendered={rendered}");
    assert!(
        rendered.contains("Please validate the restart"),
        "rendered={rendered}"
    );
}
