#![cfg(unix)]

use std::ffi::OsStr;
use std::time::Duration;

use jcode_dap::{AdapterCommand, AdapterProcess, DapError, ProcessStatus, controlled_environment};

#[test]
fn controlled_environment_has_only_allowlisted_non_secret_keys() {
    let environment = controlled_environment(Some(OsStr::new("/safe/bin")));
    let allowed = [
        "HOME",
        "USER",
        "LOGNAME",
        "TMPDIR",
        "TEMP",
        "TMP",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "SYSTEMROOT",
        "WINDIR",
        "PATH",
    ];
    assert!(
        environment
            .keys()
            .all(|key| allowed.iter().any(|allowed| key == OsStr::new(allowed)))
    );
    assert_eq!(
        environment.get(OsStr::new("PATH")).unwrap(),
        OsStr::new("/safe/bin")
    );
}

#[tokio::test]
async fn rejects_non_absolute_commands() {
    let error = AdapterProcess::spawn(&AdapterCommand::new("sh", "/"))
        .await
        .err()
        .unwrap();
    assert!(matches!(error, DapError::InvalidMessage(_)));
}

#[tokio::test]
async fn rejects_non_absolute_working_directories() {
    let error = AdapterProcess::spawn(&AdapterCommand::new("/bin/sh", "relative"))
        .await
        .err()
        .unwrap();
    assert!(matches!(error, DapError::InvalidMessage(_)));
}

#[tokio::test]
async fn captures_only_the_bounded_stderr_tail_and_reports_status() {
    let process = AdapterProcess::spawn(
        &AdapterCommand::new("/bin/sh", "/")
            .with_arg("-c")
            .with_arg("printf 123456789 >&2")
            .with_stderr_limit(4),
    )
    .await
    .unwrap();
    for _ in 0..50 {
        if matches!(
            process.status().await.unwrap(),
            ProcessStatus::Exited { .. }
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(process.recent_stderr(), b"6789");
    assert_eq!(process.stderr_capture_error(), None);
    assert!(matches!(
        process.status().await.unwrap(),
        ProcessStatus::Exited { code: Some(0) }
    ));
}

#[tokio::test]
async fn graceful_termination_stops_an_owned_process() {
    let process = AdapterProcess::spawn(
        &AdapterCommand::new("/bin/sh", "/")
            .with_arg("-c")
            .with_arg("sleep 30"),
    )
    .await
    .unwrap();
    assert_eq!(process.status().await.unwrap(), ProcessStatus::Running);
    assert!(matches!(
        process.terminate(Duration::from_secs(1)).await.unwrap(),
        ProcessStatus::Exited { .. }
    ));
}

#[tokio::test]
async fn forced_group_cleanup_removes_descendants() {
    let process = AdapterProcess::spawn(
        &AdapterCommand::new("/bin/sh", "/")
            .with_arg("-c")
            .with_arg("trap '' TERM; sleep 30 & echo $! >&2; wait"),
    )
    .await
    .unwrap();
    let mut descendant = None;
    for _ in 0..100 {
        let stderr = String::from_utf8_lossy(&process.recent_stderr())
            .trim()
            .to_owned();
        if let Ok(pid) = stderr.parse::<i32>() {
            descendant = Some(pid);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let descendant = descendant.expect("descendant PID should be reported within one second");
    process.terminate(Duration::from_millis(30)).await.unwrap();
    for _ in 0..50 {
        // SAFETY: signal 0 only checks whether this observed child PID still exists.
        if unsafe { libc::kill(descendant, 0) } != 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("descendant process {descendant} survived group cleanup");
}

#[tokio::test]
async fn drop_backstop_removes_owned_descendants() {
    let process = AdapterProcess::spawn(
        &AdapterCommand::new("/bin/sh", "/")
            .with_arg("-c")
            .with_arg("sleep 30 & echo $! >&2; wait"),
    )
    .await
    .unwrap();
    let mut descendant = None;
    for _ in 0..100 {
        let stderr = String::from_utf8_lossy(&process.recent_stderr())
            .trim()
            .to_owned();
        if let Ok(pid) = stderr.parse::<i32>() {
            descendant = Some(pid);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let descendant = descendant.expect("descendant PID should be reported within one second");
    drop(process);
    for _ in 0..50 {
        // SAFETY: signal 0 only checks whether this observed child PID still exists.
        if unsafe { libc::kill(descendant, 0) } != 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("descendant process {descendant} survived AdapterProcess drop");
}
