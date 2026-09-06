use crate::protocol::ServerEvent;
use anyhow::Result;
use tokio::sync::{mpsc, oneshot};

pub(super) type ProcessingCompletion = (u64, Result<()>, Option<String>, oneshot::Sender<bool>);

/// Preserve stream ordering while ensuring a client observing Done can submit
/// its next turn. An explicit rejection fences stale/cancelled turns. If the
/// origin disconnected, remaining attached clients still need the terminal event.
pub(super) async fn publish(
    id: u64,
    result: Result<()>,
    report: Option<String>,
    terminal: ServerEvent,
    events: &mpsc::UnboundedSender<ServerEvent>,
    completions: &mpsc::UnboundedSender<ProcessingCompletion>,
) {
    let (ready, receiver) = oneshot::channel();
    if (completions.send((id, result, report, ready))).is_err() {
        crate::logging::debug("Event recipient disconnected before delivery");
    }
    if receiver.await.unwrap_or(true) {
        if (events.send(terminal)).is_err() {
            crate::logging::debug("Event recipient disconnected before delivery");
        }
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
        ready.send(true).expect("owner ready");
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
        stale_owner.send(false).expect("reject stale completion");
        task.await.expect("publisher");
        assert!(observed.recv().await.is_none());
    }

    #[tokio::test]
    async fn advisor_completion_still_reaches_attached_clients_after_origin_disconnect() {
        let (events, mut observed) = mpsc::unbounded_channel();
        let (completions, pending) = mpsc::unbounded_channel();
        drop(pending);
        publish(
            9,
            Ok(()),
            None,
            ServerEvent::Done { id: 9 },
            &events,
            &completions,
        )
        .await;
        assert!(matches!(
            observed.recv().await,
            Some(ServerEvent::Done { id: 9 })
        ));
    }
}
