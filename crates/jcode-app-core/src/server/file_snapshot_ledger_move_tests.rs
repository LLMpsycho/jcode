use super::{FileSnapshotLedger, ReadCoverage, SessionReadFreshness, SnapshotMove};

#[tokio::test]
async fn moved_source_read_is_invalidated_even_if_the_old_path_is_recreated_identically() {
    let workspace = tempfile::tempdir().unwrap();
    let ledger = FileSnapshotLedger::new();
    let read = ledger
        .record_read(
            "reader",
            workspace.path(),
            "src/value.ts",
            b"export const value = 1;\n",
            None,
            ReadCoverage {
                ranges: Vec::new(),
                full_file: true,
            },
        )
        .await
        .unwrap();

    ledger
        .record_move_with_writes(
            "writer",
            workspace.path(),
            SnapshotMove {
                source_relative_path: "src/value.ts".to_owned(),
                expected_revision: read.revision,
                destination_relative_path: "src/renamed.ts".to_owned(),
                contents: b"export const value = 1;\n".to_vec(),
                mtime_ns: None,
            },
            Vec::new(),
        )
        .await
        .unwrap();
    assert!(matches!(
        ledger
            .check_session_read("reader", workspace.path(), "src/value.ts")
            .await
            .unwrap(),
        SessionReadFreshness::NoRead
    ));

    ledger
        .observe_text(
            workspace.path(),
            "src/value.ts",
            b"export const value = 1;\n",
            None,
        )
        .await
        .unwrap();
    assert!(matches!(
        ledger
            .check_session_read("reader", workspace.path(), "src/value.ts")
            .await
            .unwrap(),
        SessionReadFreshness::NoRead
    ));
}
