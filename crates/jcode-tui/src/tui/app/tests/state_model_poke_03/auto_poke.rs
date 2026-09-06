#[test]
fn test_poke_arms_auto_poke_until_todos_are_done() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        crate::todo::save_todos(
            &app.session.id,
            &[crate::todo::TodoItem {
                group: None,
                id: "todo-1".to_string(),
                content: "Finish the remaining task".to_string(),
                status: "pending".to_string(),
                priority: "high".to_string(),
                blocked_by: Vec::new(),
                assigned_to: None,
                confidence: None,
                completion_confidence: None,
                confidence_history: Vec::new(),
            }],
        )
        .expect("save todos");

        assert!(super::commands::handle_session_command(&mut app, "/poke"));

        assert!(app.auto_poke_incomplete_todos);
        assert!(app.pending_turn);
        assert!(app.display_messages().iter().any(|msg| {
            msg.content
                .contains("1 incomplete todo. We poked the agent")
                && msg.content.contains("/poke off")
        }));
    });
}
#[test]
fn test_poke_status_reports_current_state() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        crate::todo::save_todos(
            &app.session.id,
            &[crate::todo::TodoItem {
                group: None,
                id: "todo-1".to_string(),
                content: "Finish the remaining task".to_string(),
                status: "pending".to_string(),
                priority: "high".to_string(),
                blocked_by: Vec::new(),
                assigned_to: None,
                confidence: None,
                completion_confidence: None,
                confidence_history: Vec::new(),
            }],
        )
        .expect("save todos");

        assert!(super::commands::handle_session_command(
            &mut app,
            "/poke status"
        ));
        assert!(
            app.display_messages()
                .iter()
                .any(|msg| { msg.content.contains("Auto-poke: ON. 1 incomplete todo.") })
        );

        app.auto_poke_incomplete_todos = true;
        app.is_processing = true;
        app.queued_messages
            .push(super::commands::build_poke_message(
                &super::commands::incomplete_poke_todos(&app),
            ));
        app.hidden_queued_system_messages.push(
            "All todos are done. Todo confidence summary:\n- Weighted completion confidence: 80%."
                .to_string(),
        );

        assert!(super::commands::handle_session_command(
            &mut app,
            "/poke status"
        ));
        assert!(app.display_messages().iter().any(|msg| {
            msg.content.contains("Auto-poke: ON. 1 incomplete todo.")
                && msg.content.contains("A follow-up poke is queued.")
                && msg.content.contains("A turn is currently running.")
        }));
    });
}
#[test]
fn test_poke_off_disarms_and_clears_queued_followup() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        crate::todo::save_todos(
            &app.session.id,
            &[crate::todo::TodoItem {
                group: None,
                id: "todo-1".to_string(),
                content: "Keep going".to_string(),
                status: "pending".to_string(),
                priority: "high".to_string(),
                blocked_by: Vec::new(),
                assigned_to: None,
                confidence: None,
                completion_confidence: None,
                confidence_history: Vec::new(),
            }],
        )
        .expect("save todos");

        app.auto_poke_incomplete_todos = true;
        app.pending_queued_dispatch = true;
        app.queued_messages
            .push(super::commands::build_poke_message(
                &super::commands::incomplete_poke_todos(&app),
            ));
        app.hidden_queued_system_messages.push(
            "All todos are done. Todo confidence summary:\n- Weighted completion confidence: 80%."
                .to_string(),
        );

        assert!(super::commands::handle_session_command(
            &mut app,
            "/poke off"
        ));

        assert!(!app.auto_poke_incomplete_todos);
        assert!(!app.pending_queued_dispatch);
        assert!(app.queued_messages().is_empty());
        assert!(app.hidden_queued_system_messages.is_empty());
        assert_eq!(app.status_notice(), Some("Poke: OFF".to_string()));
        assert!(app.display_messages().iter().any(|msg| {
            msg.content.contains("Auto-poke disabled.")
                && msg.content.contains("Cleared 2 queued poke follow-ups")
        }));
    });
}
#[test]
fn test_poke_queues_when_turn_is_in_progress() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        crate::todo::save_todos(
            &app.session.id,
            &[crate::todo::TodoItem {
                group: None,
                id: "todo-1".to_string(),
                content: "Finish the remaining task".to_string(),
                status: "pending".to_string(),
                priority: "high".to_string(),
                blocked_by: Vec::new(),
                assigned_to: None,
                confidence: None,
                completion_confidence: None,
                confidence_history: Vec::new(),
            }],
        )
        .expect("save todos");

        app.is_processing = true;

        assert!(super::commands::handle_session_command(&mut app, "/poke"));

        assert!(app.auto_poke_incomplete_todos);
        assert!(app.is_processing);
        assert!(!app.cancel_requested);
        assert!(!app.pending_turn);
        assert_eq!(
            app.status_notice(),
            Some("Poke queued after current turn".to_string())
        );
        assert!(app.queued_messages().is_empty());
        assert!(app.display_messages().iter().any(|msg| {
            msg.content
                .contains("Poke queued. We'll re-check for unfinished todos after this turn")
        }));

        crate::todo::save_todos(
            &app.session.id,
            &[
                crate::todo::TodoItem {
                    group: None,
                    id: "todo-1".to_string(),
                    content: "Finish the remaining task".to_string(),
                    status: "pending".to_string(),
                    priority: "high".to_string(),
                    blocked_by: Vec::new(),
                    assigned_to: None,
                    confidence: None,
                    completion_confidence: None,
                    confidence_history: Vec::new(),
                },
                crate::todo::TodoItem {
                    group: None,
                    id: "todo-2".to_string(),
                    content: "Pick up the newly discovered task".to_string(),
                    status: "pending".to_string(),
                    priority: "medium".to_string(),
                    blocked_by: Vec::new(),
                    assigned_to: None,
                    confidence: None,
                    completion_confidence: None,
                    confidence_history: Vec::new(),
                },
            ],
        )
        .expect("save updated todos");

        super::local::finish_turn(&mut app);

        assert!(app.pending_queued_dispatch);
        assert_eq!(app.queued_messages().len(), 1);
        assert!(app.queued_messages()[0].contains("You have 2 incomplete todos"));
        assert!(!app.queued_messages()[0].contains("Pick up the newly discovered task"));
        assert!(!app.queued_messages()[0].contains("/poke off"));
    });
}
#[test]
fn test_btw_forks_even_when_turn_is_in_progress() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.is_processing = true;

        assert!(super::commands::handle_session_command(
            &mut app,
            "/btw should this fork context?"
        ));

        assert!(app.is_processing, "parent turn should keep running");
        assert!(app.queued_messages().is_empty());
        assert!(app.hidden_queued_system_messages.is_empty());
        assert!(app.display_messages().iter().any(|msg| {
            msg.content.contains("created for the next prompt")
                || msg.content.contains("Next prompt launched in")
        }));
    });
}
#[test]
fn test_finish_turn_auto_pokes_again_when_todos_remain() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        crate::todo::save_todos(
            &app.session.id,
            &[crate::todo::TodoItem {
                group: None,
                id: "todo-1".to_string(),
                content: "Keep going".to_string(),
                status: "in_progress".to_string(),
                priority: "high".to_string(),
                blocked_by: Vec::new(),
                assigned_to: None,
                confidence: None,
                completion_confidence: None,
                confidence_history: Vec::new(),
            }],
        )
        .expect("save todos");

        app.auto_poke_incomplete_todos = true;
        app.is_processing = true;
        super::local::finish_turn(&mut app);

        assert!(app.pending_queued_dispatch);
        assert_eq!(app.queued_messages().len(), 1);
        assert!(app.queued_messages()[0].contains("Continue working, or update the todo tool."));
    });
}
#[test]
fn test_finish_turn_auto_poke_queues_confidence_summary_when_todos_done() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        crate::todo::save_todos(
            &app.session.id,
            &[
                crate::todo::TodoItem {
                    group: None,
                    id: "todo-1".to_string(),
                    content: "Finish risky provider path".to_string(),
                    status: "completed".to_string(),
                    priority: "high".to_string(),
                    blocked_by: Vec::new(),
                    assigned_to: None,
                    confidence: Some(crate::todo::ConfidenceState::from_legacy_score(70)),
                    completion_confidence: Some(crate::todo::ConfidenceState::from_legacy_score(
                        80,
                    )),
                    confidence_history: Vec::new(),
                },
                crate::todo::TodoItem {
                    group: None,
                    id: "todo-2".to_string(),
                    content: "Document straightforward behavior".to_string(),
                    status: "completed".to_string(),
                    priority: "medium".to_string(),
                    blocked_by: Vec::new(),
                    assigned_to: None,
                    confidence: Some(crate::todo::ConfidenceState::from_legacy_score(90)),
                    completion_confidence: Some(crate::todo::ConfidenceState::from_legacy_score(
                        95,
                    )),
                    confidence_history: Vec::new(),
                },
            ],
        )
        .expect("save todos");

        crate::todo::save_goals(
            &app.session.id,
            &[crate::todo::TodoGoal {
                delivery_state: Some(crate::todo::DeliveryState::WorkflowValidated),
                autonomy: Some(crate::todo::Autonomy::NecessaryFollowthrough),
                iteration_maturity: Some(crate::todo::IterationMaturity::OutcomeReached),
                closed_feedback_loop: Some(crate::todo::FeedbackLoopState::Closed),
                feedback_loop: Some("verify completed work".to_string()),
                feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Representative),
                feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::MainPaths),
                feedback_loop_traceability: Some(crate::todo::FeedbackLoopTraceability::Complete),
                ..Default::default()
            }],
        )
        .expect("save goal delivery state");

        app.auto_poke_incomplete_todos = true;
        app.is_processing = true;
        super::local::finish_turn(&mut app);

        assert!(app.auto_poke_incomplete_todos);
        assert!(app.pending_queued_dispatch);
        assert_eq!(app.queued_messages.len(), 1);
        let summary = app.queued_messages[0].clone();
        let summary = &summary;
        assert!(super::commands::is_poke_message(summary));
        assert!(super::commands::is_todo_confidence_summary_message(summary));
        assert!(summary.starts_with(crate::todo::TODO_COMPLETION_CONTINUATION_MESSAGE));
        // The continuation self-identifies as an automated follow-up so the model
        // does not mistake it for a user message, but never discloses private
        // calibration details.
        assert!(summary.starts_with("[auto] "));
        assert!(!summary.to_ascii_lowercase().contains("threshold"));
        // The model is told exactly which completed todos to recheck.
        assert!(summary.contains("Finish risky provider path"));
        assert!(summary.contains("Document straightforward behavior"));
        assert!(
            app.display_messages()
                .iter()
                .any(|msg| msg.content.contains("Double-checking confidence"))
        );

        // Dispatching the follow-up does not disarm the gate. If the model
        // finishes another turn without improving completion confidence, the
        // same validation follow-up is queued again.
        app.queued_messages.clear();
        app.pending_queued_dispatch = false;
        app.is_processing = true;
        super::local::finish_turn(&mut app);
        assert!(app.auto_poke_incomplete_todos);
        assert!(app.pending_queued_dispatch);
        assert_eq!(app.queued_messages.len(), 1);

        // Once the model records sufficient completion confidence through the
        // todo tool, the next completion check requests one clean final answer.
        let mut validated = crate::todo::load_todos(&app.session.id).expect("load todos");
        for todo in &mut validated {
            todo.completion_confidence = Some(crate::todo::ConfidenceState::from_legacy_score(100));
            todo.confidence_history = match todo.id.as_str() {
                "todo-1" => vec![
                    crate::todo::ConfidenceState::Speculative,
                    crate::todo::ConfidenceState::Plausible,
                    crate::todo::ConfidenceState::Validated,
                    crate::todo::ConfidenceState::Verified,
                ],
                _ => vec![
                    crate::todo::ConfidenceState::Validated,
                    crate::todo::ConfidenceState::Verified,
                ],
            };
        }
        crate::todo::save_todos(&app.session.id, &validated).expect("save validated todos");
        app.queued_messages.clear();
        app.pending_queued_dispatch = false;
        app.is_processing = true;
        super::local::finish_turn(&mut app);
        // Auto-poke is default-on, so a completed cycle re-arms for the next
        // batch of work rather than silently switching the feature off.
        assert_eq!(app.auto_poke_incomplete_todos, app.auto_poke_default_on);
        assert!(app.pending_queued_dispatch);
        assert_eq!(
            app.queued_messages,
            vec![crate::todo::TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE.to_string()]
        );
        assert!(app.hidden_queued_system_messages.is_empty());
        assert!(app.display_messages().iter().any(|msg| {
            msg.content
                .contains("All todos done. Completion confidence: verified.")
        }));

        // The final-answer turn itself must not enqueue another final-answer
        // turn, otherwise a successfully completed cycle loops forever.
        app.queued_messages.clear();
        app.pending_queued_dispatch = false;
        app.is_processing = true;
        super::local::finish_turn(&mut app);
        assert!(!app.pending_queued_dispatch);
        assert!(app.queued_messages.is_empty());
    });
}
#[test]
fn test_todo_completion_gate_detects_abrupt_confidence_increase() {
    let summary = super::commands::todo_confidence_summary(&[crate::todo::TodoItem {
        status: "completed".to_string(),
        priority: "high".to_string(),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(0)),
        completion_confidence: Some(crate::todo::ConfidenceState::from_legacy_score(100)),
        confidence_history: vec![
            crate::todo::ConfidenceState::from_legacy_score(0),
            crate::todo::ConfidenceState::from_legacy_score(100),
        ],
        ..Default::default()
    }]);

    assert_eq!(summary.completion_average, Some(100));
    assert!(!summary.completion_confidence_needs_validation);
    assert!(summary.confidence_spike_detected);
    assert!(summary.needs_more_work);
}
#[test]
fn test_todo_completion_gate_allows_evidence_backed_confidence_steps() {
    let summary = super::commands::todo_confidence_summary(&[crate::todo::TodoItem {
        status: "completed".to_string(),
        priority: "high".to_string(),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(100)),
        completion_confidence: Some(crate::todo::ConfidenceState::from_legacy_score(100)),
        confidence_history: vec![
            crate::todo::ConfidenceState::Speculative,
            crate::todo::ConfidenceState::Plausible,
            crate::todo::ConfidenceState::Validated,
            crate::todo::ConfidenceState::Verified,
        ],
        ..Default::default()
    }]);

    assert_eq!(summary.completion_average, Some(100));
    assert!(!summary.completion_confidence_needs_validation);
    assert!(!summary.confidence_spike_detected);
    assert!(!summary.needs_more_work);
}
#[test]
fn test_finish_turn_challenges_confidence_spike_once() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        crate::todo::save_todos(
            &app.session.id,
            &[crate::todo::TodoItem {
                id: "todo-1".to_string(),
                content: "Validate provider result".to_string(),
                status: "completed".to_string(),
                priority: "high".to_string(),
                confidence: Some(crate::todo::ConfidenceState::from_legacy_score(100)),
                completion_confidence: Some(crate::todo::ConfidenceState::from_legacy_score(100)),
                confidence_history: vec![
                    crate::todo::ConfidenceState::from_legacy_score(70),
                    crate::todo::ConfidenceState::from_legacy_score(100),
                ],
                ..Default::default()
            }],
        )
        .expect("save todos");

        crate::todo::save_goals(
            &app.session.id,
            &[crate::todo::TodoGoal {
                delivery_state: Some(crate::todo::DeliveryState::WorkflowValidated),
                autonomy: Some(crate::todo::Autonomy::NecessaryFollowthrough),
                iteration_maturity: Some(crate::todo::IterationMaturity::OutcomeReached),
                feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Representative),
                feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::MainPaths),
                feedback_loop_traceability: Some(crate::todo::FeedbackLoopTraceability::Complete),
                ..Default::default()
            }],
        )
        .expect("save passing goal");

        app.auto_poke_incomplete_todos = true;
        app.is_processing = true;
        super::local::finish_turn(&mut app);

        assert!(app.auto_poke_incomplete_todos);
        assert!(app.todo_confidence_spike_challenged);
        assert!(app.pending_queued_dispatch);
        assert_eq!(app.queued_messages.len(), 1);
        assert!(
            app.queued_messages[0]
                .starts_with(crate::todo::TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE)
        );
        assert!(
            app.display_messages()
                .iter()
                .any(|msg| { msg.content.contains("Double-checking confidence jumps") })
        );

        app.queued_messages.clear();
        app.pending_queued_dispatch = false;
        app.is_processing = true;
        super::local::finish_turn(&mut app);

        assert!(app.auto_poke_incomplete_todos);
        assert!(app.todo_confidence_spike_challenged);
        assert!(app.pending_queued_dispatch);
        assert_eq!(
            app.queued_messages,
            vec![crate::todo::TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE.to_string()]
        );

        // Finishing the synthetic final-response turn must not challenge the
        // same unchanged confidence history again.
        app.queued_messages.clear();
        app.pending_queued_dispatch = false;
        app.is_processing = true;
        super::local::finish_turn(&mut app);

        assert!(app.auto_poke_incomplete_todos);
        assert!(app.todo_confidence_spike_challenged);
        assert!(!app.pending_queued_dispatch);
        assert!(app.queued_messages.is_empty());
        assert_eq!(
            app.display_messages()
                .iter()
                .filter(|message| message.content.contains("All todos done"))
                .count(),
            1
        );
    });
}
#[test]
fn test_todo_confidence_summary_hidden_queue_is_not_user_prompt() {
    let summary =
        "All todos are done. Todo confidence summary:\n- Weighted completion confidence: 94%."
            .to_string();

    let (user_messages, reminder, display_system_messages) =
        super::helpers::partition_queued_messages(Vec::new(), vec![summary.clone()]);

    assert!(user_messages.is_empty());
    assert!(display_system_messages.is_empty());
    assert_eq!(reminder.as_deref(), Some(summary.as_str()));
}
#[test]
fn test_finish_turn_without_auto_poke_does_not_queue_confidence_summary() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        crate::todo::save_todos(
            &app.session.id,
            &[crate::todo::TodoItem {
                group: None,
                id: "todo-1".to_string(),
                content: "Done without poke".to_string(),
                status: "completed".to_string(),
                priority: "high".to_string(),
                blocked_by: Vec::new(),
                assigned_to: None,
                confidence: Some(crate::todo::ConfidenceState::from_legacy_score(90)),
                completion_confidence: Some(crate::todo::ConfidenceState::from_legacy_score(90)),
                confidence_history: Vec::new(),
            }],
        )
        .expect("save todos");

        app.auto_poke_incomplete_todos = false;
        app.is_processing = true;
        super::local::finish_turn(&mut app);

        assert!(!app.pending_queued_dispatch);
        assert!(app.queued_messages().is_empty());
        assert!(
            !app.display_messages()
                .iter()
                .any(|msg| msg.content.contains("confidence summary"))
        );
    });
}
#[test]
fn test_finish_turn_auto_poke_preserves_visible_turn_started() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        crate::todo::save_todos(
            &app.session.id,
            &[crate::todo::TodoItem {
                group: None,
                id: "todo-1".to_string(),
                content: "Keep going".to_string(),
                status: "in_progress".to_string(),
                priority: "high".to_string(),
                blocked_by: Vec::new(),
                assigned_to: None,
                confidence: None,
                completion_confidence: None,
                confidence_history: Vec::new(),
            }],
        )
        .expect("save todos");

        let started = Instant::now() - Duration::from_secs(45);
        app.auto_poke_incomplete_todos = true;
        app.is_processing = true;
        app.visible_turn_started = Some(started);

        super::local::finish_turn(&mut app);

        assert_eq!(app.visible_turn_started, Some(started));
        assert!(app.pending_queued_dispatch);
    });
}
