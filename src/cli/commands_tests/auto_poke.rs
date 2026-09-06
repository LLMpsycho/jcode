#[test]
fn run_auto_poke_followup_targets_below_threshold_todos() {
    let todos = vec![
        test_todo(
            "a",
            "completed",
            "high",
            Some(ConfidenceState::Plausible),
            Some(ConfidenceState::Plausible),
        ),
        test_todo(
            "b",
            "completed",
            "low",
            Some(ConfidenceState::Plausible),
            Some(ConfidenceState::Plausible),
        ),
    ];

    let followup = build_run_auto_poke_follow_up_from_todos(&todos, false, None);

    match followup {
        Some(RunAutoPokeFollowUp::ConfidenceSummary {
            total_todos,
            message,
            ..
        }) => {
            assert_eq!(total_todos, 2);
            assert!(message.starts_with(crate::todo::TODO_COMPLETION_CONTINUATION_MESSAGE));
            assert!(message.contains("completion confidence"));
            assert!(!message.to_ascii_lowercase().contains("threshold"));
        }
        _ => panic!("expected confidence-summary follow-up"),
    }
}
#[test]
fn run_auto_poke_followup_challenges_abrupt_confidence_once() {
    let mut todo = test_todo(
        "a",
        "completed",
        "high",
        Some(ConfidenceState::Speculative),
        Some(ConfidenceState::Verified),
    );
    todo.confidence_history = vec![ConfidenceState::Speculative, ConfidenceState::Verified];

    let todos = [todo];
    match build_run_auto_poke_follow_up_from_todos(&todos, false, None) {
        Some(RunAutoPokeFollowUp::ConfidenceSummary {
            message,
            confidence_spike_challenge,
            ..
        }) => {
            assert!(confidence_spike_challenge);
            assert!(message.starts_with(crate::todo::TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE));
        }
        _ => panic!("expected confidence-spike challenge"),
    }
    assert!(build_run_auto_poke_follow_up_from_todos(&todos, true, None).is_none());
}
#[test]
fn run_auto_poke_followup_silent_when_confident_and_earned() {
    // All above threshold and no spikes: the old behavior sent an "all good"
    // summary anyway; now we spend no tokens and end the run.
    let todos = vec![
        {
            let mut todo = test_todo(
                "a",
                "completed",
                "high",
                Some(ConfidenceState::Verified),
                Some(ConfidenceState::Verified),
            );
            todo.confidence_history = vec![
                ConfidenceState::Plausible,
                ConfidenceState::Plausible,
                ConfidenceState::Validated,
                ConfidenceState::Verified,
            ];
            todo
        },
        test_todo(
            "b",
            "completed",
            "low",
            Some(ConfidenceState::Validated),
            Some(ConfidenceState::Validated),
        ),
    ];
    assert!(build_run_auto_poke_follow_up_from_todos(&todos, false, None).is_none());
}
#[test]
fn run_auto_poke_followup_prioritizes_incomplete_todos() {
    let todos = vec![
        test_todo(
            "a",
            "completed",
            "high",
            Some(ConfidenceState::Plausible),
            Some(ConfidenceState::Plausible),
        ),
        test_todo(
            "b",
            "in_progress",
            "medium",
            Some(ConfidenceState::Plausible),
            None,
        ),
    ];

    let followup = build_run_auto_poke_follow_up_from_todos(&todos, false, None);

    match followup {
        Some(RunAutoPokeFollowUp::Incomplete { count, message }) => {
            assert_eq!(count, 1);
            assert_eq!(
                message,
                "You have 1 incomplete todo. Continue working, or update the todo tool."
            );
        }
        _ => panic!("expected incomplete-todo follow-up"),
    }
}
#[test]
fn run_auto_poke_treats_completion_synonyms_and_case_as_finished() {
    for status in ["done", "finished", "complete", "Completed", " DONE "] {
        let todos = vec![test_todo(
            "a",
            status,
            "high",
            Some(ConfidenceState::Verified),
            Some(ConfidenceState::Verified),
        )];
        assert!(
            build_run_auto_poke_follow_up_from_todos(&todos, false, None).is_none(),
            "status {status:?} should not trigger an incomplete-todo poke"
        );
    }
}
#[test]
fn run_auto_poke_treats_cancelled_spelling_variants_as_finished() {
    for status in ["cancelled", "canceled", "Cancelled"] {
        let todos = vec![test_todo("a", status, "high", None, None)];
        assert!(
            build_run_auto_poke_follow_up_from_todos(&todos, false, None).is_none(),
            "status {status:?} should not trigger an incomplete-todo poke"
        );
    }
}
/// Headless `jcode run` is what the benchmarks and scripted use go through, so
/// the deferred quality review must reach that path too, not only the TUI.
#[test]
fn run_auto_poke_delivers_the_deferred_gate_digest_before_confidence() {
    let todos = vec![test_todo(
        "a",
        "completed",
        "high",
        Some(ConfidenceState::Plausible),
        Some(ConfidenceState::Plausible),
    )];
    // Without a digest, the confidence gate is what fires.
    assert!(matches!(
        build_run_auto_poke_follow_up_from_todos(&todos, false, None),
        Some(RunAutoPokeFollowUp::ConfidenceSummary { .. })
    ));
    // With one, the weak points are reviewed first, since that work can change
    // the very assessments the confidence gate judges.
    match build_run_auto_poke_follow_up_from_todos(
        &todos,
        false,
        Some("review these points".to_string()),
    ) {
        Some(RunAutoPokeFollowUp::GateDigest { message }) => {
            assert_eq!(message, "review these points");
        }
        _ => panic!("expected the gate digest to take precedence"),
    }
}
/// Open todos mean the agent is still working, so the review must wait for the
/// turn to actually end rather than interrupting mid-flight.
#[test]
fn run_auto_poke_prefers_incomplete_todos_over_the_gate_digest() {
    let todos = vec![test_todo(
        "a",
        "in_progress",
        "high",
        Some(ConfidenceState::Plausible),
        None,
    )];
    assert!(matches!(
        build_run_auto_poke_follow_up_from_todos(
            &todos,
            false,
            Some("review these points".to_string())
        ),
        Some(RunAutoPokeFollowUp::Incomplete { .. })
    ));
}
/// Regression: the digest is consumed from the log before the follow-up is
/// chosen, so a turn with open todos must not destroy it. Auto-poke iterates
/// many times with open todos on a long run, and each pass used to silently
/// discard the observations, meaning the reminder never survived to delivery.
#[test]
fn open_todos_do_not_consume_the_pending_gate_digest() {
    let _guard = crate::storage::lock_test_env();
    let previous_home = std::env::var_os("JCODE_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir.path());
    let session = "run-gate-digest-open-todos";

    crate::todo::append_gate_observations(
        session,
        &[crate::todo::GateObservation {
            kind: crate::todo::GateObservationKind::IntentUnderstanding,
            group: None,
            state: Some("partial".to_string()),
        }],
    )
    .expect("append");

    let open = vec![test_todo(
        "a",
        "in_progress",
        "high",
        Some(ConfidenceState::Plausible),
        None,
    )];
    assert!(matches!(
        build_run_auto_poke_follow_up_from_todos(
            &open,
            false,
            take_run_gate_digest_if_turn_ended(session, false, &open),
        ),
        Some(RunAutoPokeFollowUp::Incomplete { .. })
    ));
    assert!(
        !crate::todo::load_gate_observations(session)
            .expect("reload")
            .is_empty(),
        "observations must survive a poke iteration that still has open work"
    );

    // Once the work closes, the reminder is still there to deliver.
    let done = vec![test_todo(
        "a",
        "completed",
        "high",
        Some(ConfidenceState::Plausible),
        Some(ConfidenceState::Verified),
    )];
    match build_run_auto_poke_follow_up_from_todos(
        &done,
        false,
        take_run_gate_digest_if_turn_ended(session, false, &done),
    ) {
        Some(RunAutoPokeFollowUp::GateDigest { message }) => {
            assert!(message.starts_with(crate::todo::TODO_GATE_DIGEST_PREFIX));
        }
        other => panic!("expected the preserved digest to be delivered, got {other:?}"),
    }
    assert!(
        crate::todo::load_gate_observations(session)
            .expect("reload")
            .is_empty(),
        "delivering the digest should consume the log"
    );

    match previous_home {
        Some(value) => crate::env::set_var("JCODE_HOME", value),
        None => crate::env::remove_var("JCODE_HOME"),
    }
}
/// The log must be consumed on delivery, or one turn's observations would leak
/// into the next turn and be raised again against work they never described.
#[test]
fn take_run_gate_digest_consumes_the_log_and_respects_delivery() {
    let _guard = crate::storage::lock_test_env();
    let previous_home = std::env::var_os("JCODE_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir.path());
    let session = "run-gate-digest";

    crate::todo::append_gate_observations(
        session,
        &[crate::todo::GateObservation {
            kind: crate::todo::GateObservationKind::IntentUnderstanding,
            group: None,
            state: Some("partial".to_string()),
        }],
    )
    .expect("append");

    // Already delivered this turn: no second reminder, and the log is left for
    // the delivering path to have handled.
    assert!(take_run_gate_digest(session, true).is_none());

    let digest = take_run_gate_digest(session, false).expect("unresolved point should surface");
    assert!(digest.starts_with(crate::todo::TODO_GATE_DIGEST_PREFIX));
    // Consumed, so the next turn starts clean.
    assert!(
        crate::todo::load_gate_observations(session)
            .expect("reload")
            .is_empty()
    );
    assert!(take_run_gate_digest(session, false).is_none());

    match previous_home {
        Some(value) => crate::env::set_var("JCODE_HOME", value),
        None => crate::env::remove_var("JCODE_HOME"),
    }
}
#[test]
fn run_auto_poke_followup_rechecks_completion_confidence_until_it_passes() {
    let needs_validation = vec![test_todo(
        "a",
        "completed",
        "high",
        Some(ConfidenceState::Plausible),
        Some(ConfidenceState::Plausible),
    )];
    assert!(matches!(
        build_run_auto_poke_follow_up_from_todos(&needs_validation, false, None),
        Some(RunAutoPokeFollowUp::ConfidenceSummary { .. })
    ));
    assert!(matches!(
        build_run_auto_poke_follow_up_from_todos(&needs_validation, false, None),
        Some(RunAutoPokeFollowUp::ConfidenceSummary { .. })
    ));

    let validated = vec![test_todo(
        "a",
        "completed",
        "high",
        Some(ConfidenceState::Plausible),
        Some(ConfidenceState::Verified),
    )];
    assert!(matches!(
        build_run_auto_poke_follow_up_from_todos(&validated, false, None),
        Some(RunAutoPokeFollowUp::ConfidenceSummary {
            confidence_spike_challenge: true,
            ..
        })
    ));
    assert!(build_run_auto_poke_follow_up_from_todos(&validated, true, None).is_none());
}
