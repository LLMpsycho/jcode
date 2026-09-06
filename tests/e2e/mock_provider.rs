//! Mock provider for e2e tests
//!
//! Returns pre-scripted StreamEvent sequences for deterministic testing.

use anyhow::Result;
use async_stream::stream;
use jcode::message::{Message, StreamEvent, ToolDefinition};
use jcode::provider::{EventStream, Provider};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct PrivateRequest {
    pub model: String,
    pub resume_session_id: Option<String>,
}

pub struct MockProvider {
    private_session: AtomicBool,
    /// Private advisors have independent requests and never consume the main script.
    pub captured_private_requests: Arc<Mutex<Vec<PrivateRequest>>>,
    responses: Arc<Mutex<VecDeque<Vec<StreamEvent>>>>,
    models: Vec<&'static str>,
    current_model: Arc<Mutex<String>>,
    /// Captured system prompts from complete() calls (for testing)
    pub captured_system_prompts: Arc<Mutex<Vec<String>>>,
    /// Captured resume session IDs from complete() calls (for testing)
    pub captured_resume_session_ids: Arc<Mutex<Vec<Option<String>>>>,
    /// Captured model names from complete() calls (for testing)
    pub captured_models: Arc<Mutex<Vec<String>>>,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            private_session: AtomicBool::new(false),
            captured_private_requests: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(VecDeque::new())),
            models: Vec::new(),
            current_model: Arc::new(Mutex::new("mock".to_string())),
            captured_system_prompts: Arc::new(Mutex::new(Vec::new())),
            captured_resume_session_ids: Arc::new(Mutex::new(Vec::new())),
            captured_models: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn with_models(models: Vec<&'static str>) -> Self {
        let current = models
            .first()
            .map(|m| (*m).to_string())
            .unwrap_or_else(|| "mock".to_string());
        Self {
            private_session: AtomicBool::new(false),
            captured_private_requests: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(VecDeque::new())),
            models,
            current_model: Arc::new(Mutex::new(current)),
            captured_system_prompts: Arc::new(Mutex::new(Vec::new())),
            captured_resume_session_ids: Arc::new(Mutex::new(Vec::new())),
            captured_models: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Queue a response (sequence of StreamEvents) to be returned on next complete() call
    pub fn queue_response(&self, events: Vec<StreamEvent>) {
        self.responses.lock().unwrap().push_back(events);
    }
}

#[async_trait::async_trait]
impl Provider for MockProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        system: &str,
        resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        if self.private_session.load(Ordering::Acquire) {
            self.captured_private_requests
                .lock()
                .unwrap()
                .push(PrivateRequest {
                    model: self.model(),
                    resume_session_id: resume_session_id.map(str::to_owned),
                });
            return Ok(Box::pin(futures::stream::iter(vec![
                Ok(StreamEvent::TextDelta(r#"{"silence":true}"#.to_string())),
                Ok(StreamEvent::MessageEnd {
                    stop_reason: Some("end_turn".to_string()),
                }),
            ])));
        }
        // Capture only primary-session requests. Private sessions have their own evidence above.
        self.captured_system_prompts
            .lock()
            .unwrap()
            .push(system.to_string());
        self.captured_resume_session_ids
            .lock()
            .unwrap()
            .push(resume_session_id.map(|s| s.to_string()));
        self.captured_models.lock().unwrap().push(self.model());

        let events = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default();

        let stream = stream! {
            for event in events {
                yield Ok(event);
            }
        };

        Ok(Box::pin(stream))
    }

    fn name(&self) -> &str {
        "mock"
    }

    fn model(&self) -> String {
        self.current_model.lock().unwrap().clone()
    }

    fn set_model(&self, model: &str) -> Result<()> {
        if !self.models.is_empty() && !self.models.contains(&model) {
            anyhow::bail!("Unknown model: {}", model);
        }
        *self.current_model.lock().unwrap() = model.to_string();
        Ok(())
    }

    fn available_models(&self) -> Vec<&'static str> {
        self.models.clone()
    }

    fn prepare_private_session(&self) {
        self.private_session.store(true, Ordering::Release);
    }

    fn fork(&self) -> Arc<dyn Provider> {
        let current = self.current_model.lock().unwrap().clone();
        Arc::new(MockProvider {
            private_session: AtomicBool::new(self.private_session.load(Ordering::Acquire)),
            captured_private_requests: self.captured_private_requests.clone(),
            responses: self.responses.clone(),
            models: self.models.clone(),
            current_model: Arc::new(Mutex::new(current)),
            captured_system_prompts: self.captured_system_prompts.clone(),
            captured_resume_session_ids: self.captured_resume_session_ids.clone(),
            captured_models: self.captured_models.clone(),
        })
    }
}

#[tokio::test]
async fn private_forks_preserve_primary_script_model_and_resume_capture() -> Result<()> {
    use futures::StreamExt;

    let provider = MockProvider::with_models(vec!["model-a", "model-b"]);
    provider.queue_response(vec![StreamEvent::TextDelta("primary response".into())]);
    let private = provider.fork();
    private.prepare_private_session();
    private.set_model("model-b")?;
    let private_events = private
        .fork()
        .complete(&[], &[], "private", None)
        .await?
        .collect::<Vec<_>>()
        .await;
    assert!(
        matches!(&private_events[0], Ok(StreamEvent::TextDelta(text)) if text == r#"{"silence":true}"#)
    );
    assert_eq!(provider.model(), "model-a");
    assert!(
        provider
            .captured_resume_session_ids
            .lock()
            .unwrap()
            .is_empty()
    );

    let primary_events = provider
        .fork()
        .complete(&[], &[], "primary", Some("primary-resume"))
        .await?
        .collect::<Vec<_>>()
        .await;
    assert!(
        matches!(&primary_events[0], Ok(StreamEvent::TextDelta(text)) if text == "primary response")
    );
    assert_eq!(
        *provider.captured_resume_session_ids.lock().unwrap(),
        vec![Some("primary-resume".into())]
    );
    assert_eq!(*provider.captured_models.lock().unwrap(), vec!["model-a"]);
    let private_requests = provider.captured_private_requests.lock().unwrap();
    assert_eq!(private_requests.len(), 1);
    assert_eq!(private_requests[0].model, "model-b");
    assert!(private_requests[0].resume_session_id.is_none());
    Ok(())
}
