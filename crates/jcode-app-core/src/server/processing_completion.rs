use crate::protocol::ServerEvent;
use anyhow::Result;
use tokio::sync::{mpsc, oneshot};

pub(super) type ProcessingCompletion = (u64, Result<()>, Option<String>, oneshot::Sender<()>);

/// Preserve stream ordering while ensuring a client observing Done can submit
/// its next turn. Dropping the readiness sender fences stale/cancelled owners.
pub(super) async fn publish(
    id: u64,
    result: Result<()>,
    report: Option<String>,
    terminal: ServerEvent,
    events: &mpsc::UnboundedSender<ServerEvent>,
    completions: &mpsc::UnboundedSender<ProcessingCompletion>,
) {
    let (ready, receiver) = oneshot::channel();
    if completions.send((id, result, report, ready)).is_ok() && receiver.await.is_ok() {
        let _ = events.send(terminal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn advisor_followup_turn_waits_for_owner_readiness_before_done() {
        let (events, mut observed) = mpsc::unbounded_channel();
        let (completions, mut pending) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            publish(
                7,
                Ok(()),
                None,
                ServerEvent::Done { id: 7 },
                &events,
                &completions,
            )
            .await;
        });
        let (id, result, _, ready) = pending.recv().await.expect("completion");
        assert_eq!(id, 7);
        result.expect("success");
        assert!(
            observed.try_recv().is_err(),
            "Done must not race owner bookkeeping"
        );
        ready.send(()).expect("owner ready");
        task.await.expect("publisher");
        assert!(matches!(
            observed.recv().await,
            Some(ServerEvent::Done { id: 7 })
        ));
    }

    #[tokio::test]
    async fn stale_advisor_turn_completion_is_not_published() {
        let (events, mut observed) = mpsc::unbounded_channel();
        let (completions, mut pending) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            publish(
                8,
                Ok(()),
                None,
                ServerEvent::Done { id: 8 },
                &events,
                &completions,
            )
            .await;
        });
        let (_, _, _, stale_owner) = pending.recv().await.expect("completion");
        drop(stale_owner);
        task.await.expect("publisher");
        assert!(observed.recv().await.is_none());
    }
}
