// Compaction fixture keeps private advisor observations separate from primary evidence.

#[derive(Default)]
pub(crate) struct CapturingCompactionProvider {
    private_session: std::sync::atomic::AtomicBool,
    captured_messages: Arc<Mutex<Vec<Vec<Message>>>>,
}

impl Clone for CapturingCompactionProvider {
    fn clone(&self) -> Self {
        Self {
            private_session: std::sync::atomic::AtomicBool::new(
                self.private_session
                    .load(std::sync::atomic::Ordering::Acquire),
            ),
            captured_messages: Arc::clone(&self.captured_messages),
        }
    }
}

impl CapturingCompactionProvider {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn captured_messages(&self) -> Arc<Mutex<Vec<Vec<Message>>>> {
        Arc::clone(&self.captured_messages)
    }
}

#[async_trait]
impl Provider for CapturingCompactionProvider {
    async fn complete(
        &self,
        messages: &[Message],
        _tools: &[ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        if self
            .private_session
            .load(std::sync::atomic::Ordering::Acquire)
        {
            // Compaction assertions inspect primary inputs. Private advisors
            // receive independent silence and must not populate that evidence.
            return Ok(Box::pin(stream::iter(vec![
                Ok(StreamEvent::TextDelta(r#"{"silence":true}"#.to_string())),
                Ok(StreamEvent::MessageEnd {
                    stop_reason: Some("end_turn".to_string()),
                }),
            ])));
        }
        self.captured_messages
            .lock()
            .unwrap()
            .push(messages.to_vec());

        Ok(Box::pin(stream::iter(vec![
            Ok(StreamEvent::TextDelta("compaction-ok".to_string())),
            Ok(StreamEvent::MessageEnd {
                stop_reason: Some("end_turn".to_string()),
            }),
        ])))
    }

    fn name(&self) -> &str {
        "capturing-compaction"
    }

    fn supports_compaction(&self) -> bool {
        true
    }

    fn context_window(&self) -> usize {
        1_000
    }

    fn prepare_private_session(&self) {
        self.private_session
            .store(true, std::sync::atomic::Ordering::Release);
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
}

#[tokio::test]
async fn private_observations_preserve_compacted_primary_capture() -> Result<()> {
    let provider = CapturingCompactionProvider::new();
    let captured = provider.captured_messages();
    let private = provider.fork();
    private.prepare_private_session();
    let private_messages = [Message::user(
        "Private advisor context that is not primary compaction evidence",
    )];
    let private_events = private
        .fork()
        .complete(&private_messages, &[], "advisor", None)
        .await?
        .collect::<Vec<_>>()
        .await;
    assert!(
        matches!(&private_events[0], Ok(StreamEvent::TextDelta(text)) if text == r#"{"silence":true}"#)
    );
    assert!(
        captured.lock().unwrap().is_empty(),
        "private observations must not enter primary capture"
    );

    let expected = [
        "Previous Conversation Summary: completed earlier work",
        "recent preserved turn",
        "continue from the restored session",
    ];
    let primary_messages: Vec<_> = expected.iter().map(|text| Message::user(text)).collect();
    let primary_events = provider
        .fork()
        .complete(&primary_messages, &[], "primary", None)
        .await?
        .collect::<Vec<_>>()
        .await;
    assert!(
        matches!(&primary_events[0], Ok(StreamEvent::TextDelta(text)) if text == "compaction-ok")
    );
    // A later advisor observation must not change the completed primary evidence either.
    let repeated_private_events = private
        .complete(&private_messages, &[], "advisor", None)
        .await?
        .collect::<Vec<_>>()
        .await;
    assert!(repeated_private_events.iter().all(Result::is_ok));
    let captured = captured.lock().unwrap();
    assert_eq!(
        captured.len(),
        1,
        "exactly one primary completion is captured"
    );
    assert_eq!(
        captured[0]
            .iter()
            .map(flatten_text_blocks)
            .collect::<Vec<_>>(),
        expected
    );
    Ok(())
}
