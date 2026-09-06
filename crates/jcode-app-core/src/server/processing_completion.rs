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
    if receiver.await.unwrap_or(true) && events.send(terminal).is_err() {
        crate::logging::debug("Event recipient disconnected before delivery");
    }
}

/// Drain readiness handshakes while an SSH origin is disconnected. Awaiting
/// the task first would deadlock its terminal publisher on this receiver.
pub(super) async fn next_while_running(
    task: &mut tokio::task::JoinHandle<()>,
    pending: &mut mpsc::UnboundedReceiver<ProcessingCompletion>,
) -> Option<ProcessingCompletion> {
    tokio::select! {
        biased;
        completion = pending.recv() => completion,
        result = task => {
            if let Err(error) = result {
                crate::logging::warn(&format!("Disconnected turn task failed: {error}"));
            }
            None
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
    #[tokio::test]
    async fn disconnected_owner_drains_readiness_without_deadlocking_turn() {
        let (events, mut observed) = mpsc::unbounded_channel();
        let (completions, mut pending) = mpsc::unbounded_channel();
        let mut task = tokio::spawn(async move {
            publish(
                10,
                Ok(()),
                None,
                ServerEvent::Done { id: 10 },
                &events,
                &completions,
            )
            .await;
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let (id, result, _, ready) = next_while_running(&mut task, &mut pending)
                .await
                .expect("completion");
            assert_eq!(id, 10);
            result.expect("success");
            assert!(observed.try_recv().is_err());
            ready.send(true).expect("ready");
            assert!(next_while_running(&mut task, &mut pending).await.is_none());
            assert!(matches!(
                observed.recv().await,
                Some(ServerEvent::Done { id: 10 })
            ));
        })
        .await
        .expect("owner must service the handshake before joining the task");
    }
}
