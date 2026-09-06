#[tokio::test]
async fn cancel_without_local_task_still_signals_session_control() {
    let soft_interrupt_queue = Arc::new(std::sync::Mutex::new(Vec::new()));
    let stop_signal = InterruptSignal::new();
    let control = SessionControlHandle::cancel_only(
        "session_detached_cancel",
        soft_interrupt_queue,
        stop_signal.clone(),
    );
    // The point of this path is a turn this connection does not own (attach
    // after reload, server-initiated turn). Without a registered active turn
    // the cancel is a deliberate no-op, because arming the signal with nothing
    // running only kills the *next* message.
    let _active_turn = crate::turn_cancel_registry::register_active_turn(
        "session_detached_cancel",
        InterruptSignal::new(),
    );
    let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel::<ServerEvent>();
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
    let event_history = Arc::new(RwLock::new(std::collections::VecDeque::new()));
    let event_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (swarm_event_tx, _) = broadcast::channel(8);
    let mut client_is_processing = true;
    let mut message_id = Some(99);
    let mut session_id = Some("session_detached_cancel".to_string());
    let mut task = None;

    cancel_processing_message(
        &mut ProcessingState {
            client_is_processing: &mut client_is_processing,
            message_id: &mut message_id,
            session_id: &mut session_id,
            task: &mut task,
        },
        &control,
        &client_event_tx,
        &SwarmStatusRefs {
            members: &swarm_members,
            swarms_by_id: &swarms_by_id,
            event_history: &event_history,
            event_counter: &event_counter,
            event_tx: &swarm_event_tx,
        },
        Some(99),
        None,
    )
    .await;

    assert!(stop_signal.is_set());
    assert!(!client_is_processing);
    assert!(message_id.is_none());
    assert!(session_id.is_none());
    assert!(matches!(
        client_event_rx.recv().await,
        Some(ServerEvent::Interrupted)
    ));
    assert!(matches!(
        client_event_rx.recv().await,
        Some(ServerEvent::Done { id: 99 })
    ));
}
/// Regression for issue #428: the detached-turn cancel path schedules a
/// deferred reset of the shared stop signal. That reset must be epoch-guarded:
/// if a newer cancel fires during the reset window (rapid repeated Esc), the
/// stale timer must not clear it, otherwise the running turn never observes
/// the interrupt and keeps generating.
#[tokio::test]
async fn deferred_cancel_reset_does_not_erase_newer_cancel() {
    let soft_interrupt_queue = Arc::new(std::sync::Mutex::new(Vec::new()));
    let stop_signal = InterruptSignal::new();
    let control = SessionControlHandle::cancel_only(
        "session_detached_cancel_race",
        Arc::clone(&soft_interrupt_queue),
        stop_signal.clone(),
    );
    // A turn owned by another connection is what makes this the signalling
    // path rather than the idle no-op; see the sibling test.
    let _active_turn = crate::turn_cancel_registry::register_active_turn(
        "session_detached_cancel_race",
        InterruptSignal::new(),
    );
    let (client_event_tx, _client_event_rx) = mpsc::unbounded_channel::<ServerEvent>();
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
    let event_history = Arc::new(RwLock::new(std::collections::VecDeque::new()));
    let event_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (swarm_event_tx, _) = broadcast::channel(8);

    let cancel_via_no_task_path = async |request_id: u64| {
        let mut client_is_processing = true;
        let mut message_id = Some(request_id);
        let mut session_id = Some("session_detached_cancel_race".to_string());
        let mut task = None;
        cancel_processing_message(
            &mut ProcessingState {
                client_is_processing: &mut client_is_processing,
                message_id: &mut message_id,
                session_id: &mut session_id,
                task: &mut task,
            },
            &control,
            &client_event_tx,
            &SwarmStatusRefs {
                members: &swarm_members,
                swarms_by_id: &swarms_by_id,
                event_history: &event_history,
                event_counter: &event_counter,
                event_tx: &swarm_event_tx,
            },
            Some(request_id),
            None,
        )
        .await;
    };

    // First Esc: fires the signal and schedules a 500ms deferred reset.
    cancel_via_no_task_path(1).await;
    assert!(stop_signal.is_set());

    // 400ms later the user presses Esc again (turn still hasn't stopped).
    tokio::time::sleep(Duration::from_millis(400)).await;
    cancel_via_no_task_path(2).await;
    assert!(stop_signal.is_set());

    // The first press's timer expires now. It must NOT clear the second
    // press's still-unobserved cancel.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        stop_signal.is_set(),
        "stale deferred reset erased a newer cancel (issue #428)"
    );

    // The second press's own timer may still clear it afterwards.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        !stop_signal.is_set(),
        "the newest cancel's deferred reset should eventually clear the flag"
    );
}
/// Regression for issue #428: a turn actively streaming in this session but
/// NOT owned by the cancelling connection (no local task handle: post-reload
/// reattach, server-initiated wake turns, headless recovery) must abort
/// promptly even when the control handle's stop signal is a *different
/// instance* from the streaming agent's own `graceful_shutdown` signal.
///
/// Before the fix, `cancel_processing_message` hit the NO_LOCAL_TASK branch,
/// fired the stale handle-local signal (which nothing was listening to),
/// emitted `Interrupted` immediately, and the provider stream kept generating
/// for minutes ("Interrupting..." disappears, model keeps going, eventually
/// "Interrupted [x66]").
#[test]
fn cancel_aborts_detached_streaming_turn_with_stale_stop_signal() -> anyhow::Result<()> {
    let _lock = crate::storage::lock_test_env();
    let _env = IsolatedReloadRecoveryEnv::new();
    let session_id = "session_detached_streaming_cancel_428";

    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let provider: Arc<dyn Provider> = Arc::new(NeverEndingStreamProvider);
        let registry = Registry::new(Arc::clone(&provider)).await;
        let mut session =
            crate::session::Session::create_with_id(session_id.to_string(), None, None);
        session.model = Some("never-ending-stream".to_string());
        let agent = Arc::new(Mutex::new(Agent::new_with_session(
            provider, registry, session, None,
        )));

        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<ServerEvent>();

        // Start the turn the way server-initiated paths do: no entry in any
        // connection's processing-task map.
        let turn_agent = Arc::clone(&agent);
        let turn = tokio::spawn(async move {
            process_message_streaming_mpsc(turn_agent, "stream forever", Vec::new(), None, event_tx)
                .await
        });

        // Wait until the provider stream is actively producing output.
        loop {
            match tokio::time::timeout(Duration::from_secs(5), event_rx.recv()).await {
                Ok(Some(ServerEvent::TextDelta { .. })) => break,
                Ok(Some(_)) => continue,
                Ok(None) => panic!("event channel closed before streaming started"),
                Err(_) => panic!("turn never started streaming"),
            }
        }

        // Esc arrives on a connection that does not own the task. Its control
        // handle holds a stop signal instance that is NOT the streaming
        // agent's graceful_shutdown signal (stale/lost registration).
        let stale_stop_signal = InterruptSignal::new();
        let control = SessionControlHandle::cancel_only(
            session_id,
            Arc::new(std::sync::Mutex::new(Vec::new())),
            stale_stop_signal.clone(),
        );
        let (client_event_tx, _client_event_rx) = mpsc::unbounded_channel::<ServerEvent>();
        let swarm_members = Arc::new(RwLock::new(HashMap::new()));
        let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
        let event_history = Arc::new(RwLock::new(std::collections::VecDeque::new()));
        let event_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (swarm_event_tx, _) = broadcast::channel(8);
        let mut client_is_processing = false;
        let mut message_id = None;
        let mut cancel_session_id = None;
        let mut task = None;

        cancel_processing_message(
            &mut ProcessingState {
                client_is_processing: &mut client_is_processing,
                message_id: &mut message_id,
                session_id: &mut cancel_session_id,
                task: &mut task,
            },
            &control,
            &client_event_tx,
            &SwarmStatusRefs {
                members: &swarm_members,
                swarms_by_id: &swarms_by_id,
                event_history: &event_history,
                event_counter: &event_counter,
                event_tx: &swarm_event_tx,
            },
            Some(1),
            None,
        )
        .await;

        // The streaming turn must observe the cancel and stop promptly, not
        // minutes later when the provider happens to finish (issue #428).
        let result = tokio::time::timeout(Duration::from_secs(2), turn)
            .await
            .expect("streaming turn must abort promptly after cancel (issue #428)")
            .expect("turn task join");
        result.expect("cancelled turn should checkpoint cleanly");

        // The turn is over, so its cancel registration must be gone and the
        // agent's own signal must be reset so the *next* turn is not aborted
        // by the consumed cancel.
        assert!(
            crate::turn_cancel_registry::active_turn_signals(session_id).is_empty(),
            "finished turn must unregister its cancel signal"
        );
        let agent_signal = {
            let agent_guard = agent.lock().await;
            agent_guard.graceful_shutdown_signal()
        };
        assert!(
            !agent_signal.is_set(),
            "consumed cancel must not leak into the next turn"
        );
    });
    Ok(())
}
/// A cancel that arrives while the session is idle must not arm the cancel
/// signal at all.
///
/// The no-local-task branch cannot tell an idle session from one whose turn
/// another connection owns, so it used to fire the signal and clear it on a
/// 500ms timer. Any message sent inside that window began with the flag
/// already set and was aborted the instant it started: no reply, no error,
/// just a message that vanished. Pressing Esc on an idle prompt and typing
/// immediately is an ordinary thing to do, so this must be a true no-op.
#[test]
fn idle_cancel_does_not_arm_the_signal_for_the_next_turn() -> anyhow::Result<()> {
    let _lock = crate::storage::lock_test_env();
    let _env = IsolatedReloadRecoveryEnv::new();
    let session_id = "session_idle_cancel_noop";

    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let stop_signal = InterruptSignal::new();
        let control = SessionControlHandle::cancel_only(
            session_id,
            Arc::new(std::sync::Mutex::new(Vec::new())),
            stop_signal.clone(),
        );
        let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel::<ServerEvent>();
        let swarm_members = Arc::new(RwLock::new(HashMap::new()));
        let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
        let event_history = Arc::new(RwLock::new(std::collections::VecDeque::new()));
        let event_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (swarm_event_tx, _) = broadcast::channel(8);
        let mut client_is_processing = false;
        let mut message_id = None;
        let mut cancel_session_id = None;
        let mut task = None;

        assert!(
            !crate::turn_cancel_registry::has_active_turn(session_id),
            "test precondition: the session must be idle"
        );

        cancel_processing_message(
            &mut ProcessingState {
                client_is_processing: &mut client_is_processing,
                message_id: &mut message_id,
                session_id: &mut cancel_session_id,
                task: &mut task,
            },
            &control,
            &client_event_tx,
            &SwarmStatusRefs {
                members: &swarm_members,
                swarms_by_id: &swarms_by_id,
                event_history: &event_history,
                event_counter: &event_counter,
                event_tx: &swarm_event_tx,
            },
            Some(1),
            None,
        )
        .await;

        assert!(
            !stop_signal.is_set(),
            "an idle cancel must not arm the stop signal; the next turn would die instantly"
        );
        // The client still learns the cancel was handled, so a UI showing
        // "Interrupting..." resolves rather than hanging.
        match client_event_rx.try_recv() {
            Ok(ServerEvent::Interrupted) => {}
            other => panic!("idle cancel must still report Interrupted, got {other:?}"),
        }
    });
    Ok(())
}
