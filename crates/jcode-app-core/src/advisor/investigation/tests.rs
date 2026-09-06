use super::*;
use std::collections::HashSet;

struct EnvGuard(&'static str, Option<std::ffi::OsString>);

impl EnvGuard {
    fn set(key: &'static str, value: &std::ffi::OsStr) -> Self {
        let previous = std::env::var_os(key);
        crate::env::set_var(key, value);
        crate::config::invalidate_config_cache();
        Self(key, previous)
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.1.as_ref() {
            Some(value) => crate::env::set_var(self.0, value),
            None => crate::env::remove_var(self.0),
        }
        crate::config::invalidate_config_cache();
    }
}

async fn reader(root: &Path, session: &str) -> AdvisorInvestigation {
    AdvisorInvestigation::new(
        Registry::advisor_test_registry().await,
        session.into(),
        root.into(),
    )
    .expect("reader")
}

#[tokio::test]
async fn investigation_reads_unprovided_code_and_redacts_bounded_results() {
    let _guard = crate::storage::lock_test_env();
    let dir = tempfile::tempdir().expect("workspace");
    std::fs::write(dir.path().join("logic.rs"), "fn acceptance_bug() { return false; }\nOPENAI_API_KEY=sk-private-investigation-token-123456789\n").expect("code");
    let reader = reader(dir.path(), "investigation-read").await;
    let definitions = reader.definitions().await;
    assert!(
        definitions
            .iter()
            .any(|definition| definition.name == "read")
    );
    assert!(
        definitions
            .iter()
            .any(|definition| definition.name == "agentgrep")
    );
    let output = reader
        .execute("read", &json!({"file_path":"logic.rs"}))
        .await
        .expect("read");
    assert!(output.contains("acceptance_bug"));
    assert!(!output.contains("sk-private-investigation"));
    std::fs::write(
        dir.path().join("long.rs"),
        "let line = 1234567890;\n".repeat(20_000),
    )
    .expect("long file");
    let output = reader
        .execute("read", &json!({"file_path":"long.rs","limit":99999999}))
        .await
        .expect("bounded read");
    assert!(output.len() <= MAX_RESULT_BYTES);
    assert!(!output.contains("  201\t"));
    std::fs::write(
        dir.path().join("wide.rs"),
        format!("{}\n", "x".repeat(1000)).repeat(200),
    )
    .expect("wide file");
    let output = reader
        .execute("read", &json!({"file_path":"wide.rs"}))
        .await
        .expect("wide read");
    assert!(output.len() <= MAX_RESULT_BYTES && output.contains("excerpt truncated"));
}

#[tokio::test]
async fn investigation_denies_effects_unknown_tools_and_revoked_parent_grants() {
    let _guard = crate::storage::lock_test_env();
    let dir = tempfile::tempdir().expect("workspace");
    std::fs::write(dir.path().join("code.rs"), "original").expect("code");
    let reader = reader(dir.path(), "investigation-deny").await;
    for name in [
        "write",
        "edit",
        "bash",
        "shell_exec",
        "batch",
        "mcp__reader",
        "nonexistent",
    ] {
        assert!(
            reader
                .execute(name, &json!({"file_path":"code.rs","content":"changed"}))
                .await
                .is_err(),
            "{name}"
        );
    }
    crate::tool::set_session_tool_policy(
        "investigation-deny",
        None,
        HashSet::from(["read".into()]),
    );
    assert!(reader.definitions().await.is_empty());
    assert!(
        reader
            .execute("Read", &json!({"file_path":"code.rs"}))
            .await
            .is_err()
    );
    assert!(
        reader
            .execute("file_grep", &json!({"query":"original"}))
            .await
            .is_err()
    );
    crate::tool::clear_session_tool_policy("investigation-deny");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("code.rs")).expect("unchanged"),
        "original"
    );
}

#[tokio::test]
async fn investigation_confines_paths_and_rejects_credential_stores() {
    let _guard = crate::storage::lock_test_env();
    let dir = tempfile::tempdir().expect("workspace");
    let outside = tempfile::tempdir().expect("outside");
    std::fs::write(outside.path().join("outside.rs"), "outside secret").expect("outside");
    let reader = reader(dir.path(), "investigation-confined").await;
    assert!(
        reader
            .execute(
                "read",
                &json!({"file_path":outside.path().join("outside.rs")})
            )
            .await
            .is_err()
    );
    assert!(
        reader
            .execute(
                "agentgrep",
                &json!({"query":"secret","path":outside.path()})
            )
            .await
            .is_err()
    );
    for name in [
        ".env",
        ".env.local",
        "credentials.json",
        "secrets.toml",
        "private.pem",
        ".npmrc",
    ] {
        std::fs::write(dir.path().join(name), "opaque-secret").expect("credential");
        assert!(
            reader
                .execute("read", &json!({"file_path":name}))
                .await
                .is_err(),
            "{name}"
        );
        assert!(
            reader
                .execute("agentgrep", &json!({"query":"secret","path":name}))
                .await
                .is_err(),
            "{name}"
        );
    }
    assert!(
        reader
            .execute("agentgrep", &json!({"query":"secret","hidden":true}))
            .await
            .is_err()
    );
    assert!(
        reader
            .execute("agentgrep", &json!({"query":"secret","mode":"trace"}))
            .await
            .is_err()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn investigation_rejects_symlink_escape_and_search_does_not_follow_it() {
    let _guard = crate::storage::lock_test_env();
    let dir = tempfile::tempdir().expect("workspace");
    let outside = tempfile::tempdir().expect("outside");
    std::fs::write(outside.path().join("leak.rs"), "outside_secret_marker").expect("outside");
    std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).expect("link");
    let reader = reader(dir.path(), "investigation-link").await;
    assert!(
        reader
            .execute("read", &json!({"file_path":"linked/leak.rs"}))
            .await
            .is_err()
    );
    assert!(
        reader
            .execute(
                "agentgrep",
                &json!({"query":"outside_secret_marker","path":"linked"})
            )
            .await
            .is_err()
    );
    let output = reader
        .execute("agentgrep", &json!({"query":"outside_secret_marker"}))
        .await
        .expect("search");
    assert!(!output.contains("outside_secret_marker"));
}

#[tokio::test]
async fn investigation_searches_code_and_names_without_credential_matches() {
    let _guard = crate::storage::lock_test_env();
    let dir = tempfile::tempdir().expect("workspace");
    std::fs::create_dir(dir.path().join("src")).expect("src");
    std::fs::write(
        dir.path().join("src/logic.rs"),
        "fn requested_bug_marker() {}\n",
    )
    .expect("code");
    std::fs::write(
        dir.path().join("credentials.json"),
        "requested_bug_marker private_value\n",
    )
    .expect("credential");
    std::fs::write(
        dir.path().join("CREDENTIALS.JSON"),
        "requested_bug_marker uppercase_private_value\n",
    )
    .expect("credential");
    std::fs::create_dir(dir.path().join(".hidden")).expect("hidden directory");
    std::fs::write(
        dir.path().join(".hidden/file.rs"),
        "requested_bug_marker hidden_value\n",
    )
    .expect("hidden file");
    std::fs::write(
        dir.path().join(".env"),
        "requested_bug_marker hidden_value\n",
    )
    .expect("credential");
    let reader = reader(dir.path(), "investigation-search").await;
    let output = reader
        .execute(
            "agentgrep",
            &json!({"query":"requested_bug_marker","glob":"**/*"}),
        )
        .await
        .expect("grep");
    assert!(output.contains("logic.rs"), "{output}");
    assert!(output.contains("fn requested_bug_marker"), "{output}");
    assert!(
        !output.contains("private_value") && !output.contains("hidden_value"),
        "{output}"
    );
    let files = reader
        .execute("agentgrep", &json!({"mode":"find","query":"logic"}))
        .await
        .expect("find");
    assert!(files.contains("logic.rs"));
}

#[tokio::test]
async fn investigation_refuses_configured_policy_hooks_without_running_them() {
    let _guard = crate::storage::lock_test_env();
    let dir = tempfile::tempdir().expect("workspace");
    std::fs::write(dir.path().join("code.rs"), "code").expect("code");
    let _hook = EnvGuard::set("JCODE_HOOK_PRE_TOOL", std::ffi::OsStr::new("exit 2"));
    let reader = reader(dir.path(), "investigation-hook").await;
    assert!(reader.definitions().await.is_empty());
    assert!(
        reader
            .restriction_notice()
            .expect("notice")
            .contains("pre_tool")
    );
    let error = reader
        .execute("read", &json!({"file_path":"code.rs"}))
        .await
        .expect_err("must fail closed");
    assert!(error.to_string().contains("pre_tool policy hook"));
}

#[cfg(unix)]
#[tokio::test]
async fn investigation_search_does_not_execute_inherited_ripgrep_preprocessor() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = crate::storage::lock_test_env();
    let dir = tempfile::tempdir().expect("workspace");
    let config_dir = tempfile::tempdir().expect("config");
    let script = config_dir.path().join("preprocessor");
    let marker = config_dir.path().join("executed");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\ncat \"$1\"\n", marker.display()),
    )
    .expect("script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let config = config_dir.path().join("rg-config");
    std::fs::write(&config, format!("--pre\n{}\n--follow\n", script.display())).expect("config");
    let _config = EnvGuard::set("RIPGREP_CONFIG_PATH", config.as_os_str());
    std::fs::write(dir.path().join("logic.rs"), "actual_search_result").expect("code");
    let reader = reader(dir.path(), "investigation-rg-config").await;
    let output = reader
        .execute("agentgrep", &json!({"query":"actual_search_result"}))
        .await
        .expect("search");
    assert!(output.contains("actual_search_result"));
    assert!(!marker.exists());
}

#[test]
fn excerpts_bound_utf8_and_redact_partial_private_keys() {
    let output = bounded_excerpt(
        &format!(
            "{}-----BEGIN PRIVATE KEY-----\n{}",
            "x".repeat(100),
            "secret".repeat(20_000)
        ),
        1024,
    );
    assert!(output.len() <= 1024);
    assert!(!output.contains("secret"));
    let output = bounded_excerpt(&"é".repeat(5000), 100);
    assert!(output.len() <= 100);
    let output = bounded_excerpt(
        r#"{"db_password":"opaque password value", "client_secret":"opaque\\value", "api_key":"not-vendor-shaped", "acceptance":"still visible"}"#,
        1024,
    );
    assert!(!output.contains("opaque") && !output.contains("not-vendor-shaped"));
    assert!(output.contains("still visible"));
    let truncated_json = bounded_excerpt(r#"{"authorization":"opaque-unfinished-value"#, 1024);
    assert!(!truncated_json.contains("opaque-unfinished-value"));
    let large_arguments = bounded_json_excerpt(
        &json!({"password":"opaque-value", "source":"x".repeat(200_000)}),
        512,
    );
    assert!(large_arguments.len() <= 512 && !large_arguments.contains("opaque-value"));
}
