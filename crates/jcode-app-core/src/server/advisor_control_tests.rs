use super::*;
use crate::message::{Message, ToolDefinition};
use crate::provider::EventStream;
use async_trait::async_trait;

struct ControlProvider;

#[async_trait]
impl Provider for ControlProvider {
    fn name(&self) -> &str { "advisor-control-fixture" }
    fn fork(&self) -> Arc<dyn Provider> { Arc::new(Self) }
    async fn complete(&self, _: &[Message], _: &[ToolDefinition], _: &str, _: Option<&str>) -> anyhow::Result<EventStream> {
        anyhow::bail!("advisor controls must not call a provider")
    }
}

async fn test_agent(session: &str) -> Arc<Mutex<Agent>> {
    let provider: Arc<dyn Provider> = Arc::new(ControlProvider);
    let registry = crate::tool::Registry::new(Arc::clone(&provider)).await;
    let session = crate::session::Session::create_with_id(session.into(), None, None);
    Arc::new(Mutex::new(Agent::new_with_session(provider, registry, session, None)))
}

async fn next_result(events: &mut mpsc::UnboundedReceiver<ServerEvent>, expected_id: u64) -> AdvisorControlResult {
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv()).await.expect("control result arrives").expect("event channel");
    let ServerEvent::AdvisorResult { id, result } = event else { panic!("expected advisor result") };
    assert_eq!(id, expected_id);
    result
}

#[tokio::test]
async fn advisor_model_options_do_not_block_disable_or_status_during_primary_turn() {
    let agent = test_agent("advisor_busy_options").await;
    let manager = Arc::new(AdvisorManager::default());
    let (events, mut receiver) = mpsc::unbounded_channel();
    let busy = agent.lock().await;
    handle_with_manager(1, AdvisorRequest::ModelOptions { selection: None }, "advisor_busy_options", &agent, &events, Arc::clone(&manager));
    assert!(receiver.try_recv().is_err());
    handle_with_manager(2, AdvisorRequest::Disable, "advisor_busy_options", &agent, &events, Arc::clone(&manager));
    assert!(next_result(&mut receiver, 2).await.message.contains("disabled"));
    handle_with_manager(3, AdvisorRequest::Status, "advisor_busy_options", &agent, &events, Arc::clone(&manager));
    let status = next_result(&mut receiver, 3).await;
    assert!(!status.model_settings.expect("saved status").enabled);
    drop(busy);
    let options = next_result(&mut receiver, 1).await;
    assert!(options.model_options.is_some());
    assert!(!options.model_settings.expect("current status").enabled);
}

#[tokio::test]
async fn advisor_deferred_selection_cannot_reenable_after_later_disable() {
    let agent = test_agent("advisor_busy_select").await;
    let manager = Arc::new(AdvisorManager::default());
    let (events, mut receiver) = mpsc::unbounded_channel();
    let busy = agent.lock().await;
    handle_with_manager(1, AdvisorRequest::UsePrimary, "advisor_busy_select", &agent, &events, Arc::clone(&manager));
    handle_with_manager(2, AdvisorRequest::Disable, "advisor_busy_select", &agent, &events, Arc::clone(&manager));
    next_result(&mut receiver, 2).await;
    drop(busy);
    let selected = next_result(&mut receiver, 1).await;
    assert!(selected.error.expect("superseded request").contains("superseded"));
    assert!(!manager.is_enabled("advisor_busy_select", true));
}

#[test]
fn advisor_legacy_status_and_error_responses_keep_the_message_contract() {
    let manager = AdvisorManager::default();
    let config = AdvisorConfig::default();
    let status = control_request(&manager, "status", &config, AdvisorRequest::Status);
    assert!(status.message.starts_with("Advisor: off"));
    assert!(status.message.contains("model and effort follow primary"));
    let unavailable = crate::provider::RouteSelection {
        model: "missing".into(), runtime_key: crate::provider::RuntimeKey::OpenAIOAuth,
        api_method: "openai-oauth".into(), provider_label: "OpenAI".into(), detail: String::new(),
    };
    let result = model_request(&manager, "status", &ControlProvider, &config, AdvisorRequest::SelectModel { selection: unavailable, reasoning_effort: Some("high".into()) }, manager.begin_model_selection("status"));
    assert!(result.error.is_some());
    assert!(!result.model_settings.expect("settings").enabled);
}
