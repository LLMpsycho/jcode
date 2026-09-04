use std::sync::Arc;

use super::*;

fn deadline_fixture() -> Fixture {
    fixture_with_operations(
        "owner",
        DebugOperationConfig {
            operation_timeout: Duration::from_millis(30),
            ..Default::default()
        },
    )
}

async fn expire_deadline() {
    tokio::time::advance(Duration::from_millis(31)).await;
    tokio::task::yield_now().await;
}

fn spawn_set(
    f: &Fixture,
    line: u64,
) -> tokio::task::JoinHandle<Result<DebugBreakpointMutationResult>> {
    let manager = f.manager.clone();
    let id = f.id;
    let source = f.source.clone();
    tokio::spawn(async move {
        manager
            .set_breakpoint(
                "owner",
                id,
                DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(line)),
            )
            .await
    })
}

#[tokio::test(start_paused = true)]
async fn one_deadline_expires_independently_during_primary_response_wait() {
    let mut f = deadline_fixture();
    let task = spawn_set(&f, 1);
    let primary = request(&mut f.adapter).await;
    assert_eq!(primary.command, "setBreakpoints");
    expire_deadline().await;
    assert!(matches!(
        task.await.unwrap(),
        Err(DapError::RequestTimeout { .. })
    ));
}

#[tokio::test(start_paused = true)]
async fn one_deadline_expires_independently_during_response_validation() {
    let mut f = deadline_fixture();
    let entry = f.manager.core.entry(f.id).unwrap();
    let held = Arc::clone(&entry.breakpoint_validation)
        .acquire_owned()
        .await
        .unwrap();
    let task = spawn_set(&f, 1);
    let primary = request(&mut f.adapter).await;
    f.adapter
        .respond_ok(
            &primary,
            Some(json!({"breakpoints":[{"id":1,"verified":true}]})),
        )
        .await
        .unwrap();
    entry
        .breakpoint_test_gates
        .response_validation_entered
        .notified()
        .await;
    expire_deadline().await;
    assert!(
        matches!(task.await.unwrap(), Err(DapError::RequestTimeout { command }) if command == "setBreakpoints response validation")
    );
    drop(held);
    assert_eq!(
        f.manager.breakpoints("owner", f.id).unwrap().sources[0].synchronization,
        DebugBreakpointSynchronization::Indeterminate
    );
}

#[tokio::test(start_paused = true)]
async fn one_deadline_expires_independently_during_post_response_revalidation() {
    let mut f = deadline_fixture();
    let entry = f.manager.core.entry(f.id).unwrap();
    let task = spawn_set(&f, 1);
    let primary = request(&mut f.adapter).await;
    let held = Arc::clone(&entry.source_hash)
        .acquire_owned()
        .await
        .unwrap();
    f.adapter
        .respond_ok(
            &primary,
            Some(json!({"breakpoints":[{"id":1,"verified":true}]})),
        )
        .await
        .unwrap();
    entry
        .breakpoint_test_gates
        .post_response_revalidation_entered
        .notified()
        .await;
    expire_deadline().await;
    assert!(matches!(
        task.await.unwrap(),
        Err(DapError::BreakpointReconciliationIndeterminate { message, .. })
            if message == "source changed and compensating clear failed"
    ));
    drop(held);
    assert_eq!(
        f.manager.breakpoints("owner", f.id).unwrap().sources[0].synchronization,
        DebugBreakpointSynchronization::Indeterminate
    );
}

#[tokio::test(start_paused = true)]
async fn one_deadline_expires_independently_during_compensating_clear() {
    let mut f = deadline_fixture();
    let task = spawn_set(&f, 1);
    let primary = request(&mut f.adapter).await;
    std::fs::write(&f.source, b"changed").unwrap();
    f.adapter
        .respond_ok(
            &primary,
            Some(json!({"breakpoints":[{"id":1,"verified":true}]})),
        )
        .await
        .unwrap();
    let clear = request(&mut f.adapter).await;
    assert_eq!(clear.arguments.as_ref().unwrap()["breakpoints"], json!([]));
    expire_deadline().await;
    assert!(matches!(
        task.await.unwrap(),
        Err(DapError::BreakpointReconciliationIndeterminate { .. })
    ));
}

#[tokio::test(start_paused = true)]
async fn one_deadline_expires_independently_during_indeterminate_reset() {
    let mut f = deadline_fixture();
    let first = spawn_set(&f, 1);
    let primary = request(&mut f.adapter).await;
    f.adapter
        .respond_ok(&primary, Some(json!({"breakpoints":[{}]})))
        .await
        .unwrap();
    assert!(first.await.unwrap().is_err());
    let retry = spawn_set(&f, 2);
    let reset = request(&mut f.adapter).await;
    assert_eq!(reset.arguments.as_ref().unwrap()["breakpoints"], json!([]));
    expire_deadline().await;
    assert!(matches!(
        retry.await.unwrap(),
        Err(DapError::RequestTimeout { .. })
    ));
}
