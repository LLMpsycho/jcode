//! Parity between the Rust and TypeScript SDKs.
//!
//! Second-order dogfooding only works if the two SDKs stay the same shape. If
//! the Rust one drifts into "whatever external client happened to need", external client
//! stops telling us anything about the TypeScript one and we are back to
//! validating the TS SDK with examples written to make it look good.
//!
//! So the capability list below is the contract, and both SDKs are checked
//! against it: a method added to one and not the other fails here. Naming is
//! the only translation (`snake_case` in Rust, `camelCase` in TS); semantics
//! and arity are expected to match.
//!
//! What is deliberately *not* checked: error handling, streaming style, and
//! launch strategy. `Result` versus `throw`, channels versus `EventEmitter`,
//! and "attach to the user's jcode" versus "provision a private instance" are
//! places where forcing symmetry would produce an un-idiomatic SDK in one
//! language to flatter the other.

/// One capability, named in both SDKs' conventions.
struct Capability {
    /// Rust method on `JcodeClient`.
    rust: &'static str,
    /// TypeScript method on `JcodeClient`.
    ts: &'static str,
}

/// The shared surface. Adding a capability means adding it here first.
const CAPABILITIES: &[Capability] = &[
    cap("connect", "connect"),
    cap("launch", "launch"),
    cap("list_sessions", "listSessions"),
    cap("list_sessions_limited", "listSessionsLimited"),
    cap("archive_session", "archiveSession"),
    cap("restore_session", "restoreSession"),
    cap("set_retention_policy", "setRetentionPolicy"),
    cap("create_session", "createSession"),
    cap("attach_session", "attachSession"),
    cap("fork_session", "forkSession"),
    cap("detach_session", "detachSession"),
    cap("send_message", "sendMessage"),
    cap("cancel", "cancel"),
    cap("soft_interrupt", "softInterrupt"),
    cap("soft_interrupt_with_images", "softInterruptWithImages"),
    cap("get_history", "getHistory"),
    cap("get_history_with_images", "getHistoryWithImages"),
    cap("peek_session", "peekSession"),
    cap("clear", "clear"),
    cap("rewind", "rewind"),
    cap("rewind_undo", "rewindUndo"),
    cap("respond_to_permission", "respondToPermission"),
    cap("list_models", "listModels"),
    cap("get_runtime_info", "getRuntimeInfo"),
    cap("set_api_key", "setApiKey"),
    cap("clear_api_key", "clearApiKey"),
    cap("read_file", "readFile"),
    cap("find_files", "findFiles"),
    cap("search_text", "searchText"),
    cap("file_status", "fileStatus"),
    cap("set_model", "setModel"),
    cap("set_reasoning_effort", "setReasoningEffort"),
    cap("advisor", "advisor"),
    cap("compact", "compact"),
    cap("rename_session", "renameSession"),
    cap("cancel_soft_interrupts", "cancelSoftInterrupts"),
    cap("ping", "ping"),
    cap("run", "run"),
    cap("run_structured", "runStructured"),
    cap("events", "events"),
    cap("global_events", "globalEvents"),
    cap("request", "request"),
    cap("notify", "notify"),
    cap("supports", "supports"),
];

const fn cap(rust: &'static str, ts: &'static str) -> Capability {
    Capability { rust, ts }
}

/// Every shared capability exists in the Rust SDK.
#[test]
fn the_rust_sdk_implements_every_shared_capability() {
    let methods = rust_public_methods();
    let missing: Vec<&str> = CAPABILITIES
        .iter()
        .map(|c| c.rust)
        .filter(|name| !methods.iter().any(|method| method == name))
        .collect();
    assert!(
        missing.is_empty(),
        "the shared SDK surface names capabilities the Rust SDK does not have: \
         {missing:?}. Implement them in a declared JcodeClient module, or remove them from \
         CAPABILITIES if the capability is being dropped from both SDKs."
    );
}

/// Every shared capability exists in the TypeScript SDK.
#[test]
fn the_typescript_sdk_implements_every_shared_capability() {
    let Some(methods) = ts_public_methods() else {
        // Vendored builds without the sdk/ tree: nothing to compare against.
        return;
    };
    let missing: Vec<&str> = CAPABILITIES
        .iter()
        .map(|c| c.ts)
        .filter(|name| !methods.iter().any(|method| method == name))
        .collect();
    assert!(
        missing.is_empty(),
        "the shared SDK surface names capabilities the TypeScript SDK does not \
         have: {missing:?}. A capability that exists only in Rust means \
         external client is exercising a design the shipped SDK does not have, which \
         is the drift this test exists to prevent."
    );
}

/// Neither SDK has a public capability that is missing from the shared list.
///
/// The direction that actually rots: someone adds a method to the Rust SDK for
/// external client, never touches the TS SDK, and the lists silently diverge. Failing
/// here forces the decision to be made rather than deferred.
#[test]
fn neither_sdk_has_an_untriaged_public_capability() {
    let rust = rust_public_methods();
    let known: std::collections::BTreeSet<&str> = CAPABILITIES.iter().map(|c| c.rust).collect();
    let untriaged: Vec<&String> = rust
        .iter()
        .filter(|name| !known.contains(name.as_str()))
        .filter(|name| !RUST_ONLY.contains(&name.as_str()))
        .collect();
    assert!(
        untriaged.is_empty(),
        "these public JcodeClient methods are in the Rust SDK but not in the \
         shared capability list: {untriaged:?}. Add each to CAPABILITIES with \
         its TypeScript counterpart, or to RUST_ONLY with a comment saying why \
         it is Rust-specific."
    );

    let Some(ts) = ts_public_methods() else {
        return;
    };
    let known: std::collections::BTreeSet<&str> = CAPABILITIES.iter().map(|c| c.ts).collect();
    let untriaged: Vec<&String> = ts
        .iter()
        .filter(|name| !known.contains(name.as_str()))
        .filter(|name| !TS_ONLY.iter().any(|(allowed, _)| allowed == &name.as_str()))
        .collect();
    assert!(
        untriaged.is_empty(),
        "these public JcodeClient methods are in the TypeScript SDK but not in the \
         shared capability list: {untriaged:?}. Add each to CAPABILITIES and the \
         Rust SDK, or to TS_ONLY with a reason when it is genuinely language-specific."
    );

    if !TS_ONLY.is_empty() {
        eprintln!(
            "warning: SDK parity has {} explicitly triaged TypeScript-only methods: {}",
            TS_ONLY.len(),
            TS_ONLY
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

/// Rust-specific members, with the reason each one is not mirrored.
const RUST_ONLY: &[&str] = &[
    // Rust's native process transport/launch strategy. TypeScript accepts a
    // caller-supplied transport; a built-in SSH launcher is not yet mirrored.
    "connect_ssh",
    // `connect_with` is the explicit transport seam Rust tests use; TypeScript
    // accepts its transport through the options passed to `connect`.
    "connect_with",
    // `Drop` closes the connection in Rust, so there is no `close()` to mirror.
    "is_closed",
    // TS reads `client.socketPath` as a field; Rust exposes it as an accessor.
    "socket_path",
];

/// Public TypeScript methods not yet represented by an equivalent Rust method.
///
/// This is intentionally noisy technical debt, not a second capability list.
/// New TS methods fail the parity test unless they are implemented in Rust or
/// added here with a reviewable reason. Removing entries is the parity roadmap.
const TS_ONLY: &[(&str, &str)] = &[("close", "Rust closes through Drop")];

/// Read the SDK's declared production modules, including extracted impls.
/// Unlinked files and test-only modules cannot satisfy a public capability.
fn rust_client_source(root: &std::path::Path) -> String {
    let mut pending = vec![root.join("lib.rs")];
    let mut visited = std::collections::BTreeSet::new();
    let mut sources = Vec::new();
    while let Some(file) = pending.pop() {
        let file = file.canonicalize().unwrap_or_else(|error| {
            panic!(
                "the declared Rust SDK source {} must exist: {error}",
                file.display()
            )
        });
        if !visited.insert(file.clone()) {
            continue;
        }
        let source = std::fs::read_to_string(&file).unwrap_or_else(|error| {
            panic!(
                "the Rust SDK source {} must be readable: {error}",
                file.display()
            )
        });
        pending.extend(declared_production_modules(&file, &source));
        sources.push(source);
    }
    sources.join("\n")
}

/// SDK modules use rustfmt's file-level declarations. Resolve both ordinary
/// `mod child;` layouts and explicit paths relative to the declaring file.
fn declared_production_modules(file: &std::path::Path, source: &str) -> Vec<std::path::PathBuf> {
    let parent = file
        .parent()
        .expect("Rust module source has a parent directory");
    let stem = file
        .file_stem()
        .and_then(|name| name.to_str())
        .expect("Rust source stem is UTF-8");
    let module_dir = if matches!(stem, "lib" | "main" | "mod") {
        parent.to_path_buf()
    } else {
        parent.join(stem)
    };
    let mut modules = Vec::new();
    let mut explicit_path = None;
    let mut test_only = false;
    for line in source.lines() {
        // Nested impl bodies and inline test modules are indented by rustfmt.
        if line.starts_with(char::is_whitespace) || line.is_empty() || line.starts_with("//") {
            continue;
        }
        if line == "#[cfg(test)]" {
            test_only = true;
            continue;
        }
        if let Some(path) = line
            .strip_prefix("#[path = \"")
            .and_then(|line| line.strip_suffix("\"]"))
        {
            explicit_path = Some(parent.join(path));
            continue;
        }
        if line.starts_with("#[") {
            continue;
        }
        let declaration = line.strip_prefix("pub ").unwrap_or(line);
        let declaration = if declaration.starts_with("pub(") {
            declaration
                .split_once(") ")
                .map(|(_, rest)| rest)
                .unwrap_or(declaration)
        } else {
            declaration
        };
        let name = declaration
            .strip_prefix("mod ")
            .and_then(|line| line.strip_suffix(';'));
        if let Some(name) = name.filter(|_| !test_only) {
            let path = explicit_path.take().unwrap_or_else(|| {
                let flat = module_dir.join(format!("{name}.rs"));
                let nested = module_dir.join(name).join("mod.rs");
                assert!(
                    !(flat.exists() && nested.exists()),
                    "ambiguous SDK module {name}"
                );
                if flat.exists() { flat } else { nested }
            });
            modules.push(path);
        }
        test_only = false;
        explicit_path = None;
    }
    modules
}

fn ts_client_source() -> Option<String> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk/typescript/src/client.ts");
    std::fs::read_to_string(path).ok()
}

/// Public method names in `impl JcodeClient`.
fn rust_public_methods() -> Vec<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    rust_public_methods_in(&root)
}

fn rust_public_methods_in(root: &std::path::Path) -> Vec<String> {
    let source = rust_client_source(root);
    let mut methods = std::collections::BTreeSet::new();
    let mut remaining = source.as_str();
    while let Some(start) = remaining.find("impl JcodeClient {") {
        let body = &remaining[start..];
        let end = body.find("\n}").unwrap_or(body.len());
        methods.extend(body[..end].lines().filter_map(|line| {
            let rest = line.trim().strip_prefix("pub fn ")?;
            Some(
                rest.chars()
                    .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                    .collect::<String>(),
            )
        }));
        remaining = &body[end.min(body.len())..];
    }
    methods.into_iter().collect()
}

/// Public method names in the TypeScript `JcodeClient` class.
///
/// Methods are two-space-indented in this file. Private helpers are explicitly
/// excluded and overloads are deduplicated. Keeping this small parser here makes
/// the guard run under ordinary `cargo test`, without requiring Node or a TS AST.
fn ts_public_methods() -> Option<Vec<String>> {
    let source = ts_client_source()?;
    let start = source.find("export class JcodeClient")?;
    let mut methods = std::collections::BTreeSet::new();
    for line in source[start..].lines() {
        let Some(mut declaration) = line.strip_prefix("  ") else {
            continue;
        };
        if declaration.starts_with(' ') || declaration.starts_with("private ") {
            continue;
        }
        declaration = declaration.strip_prefix("static ").unwrap_or(declaration);
        declaration = declaration.strip_prefix("async ").unwrap_or(declaration);
        let Some(open) = declaration.find('(') else {
            continue;
        };
        let name = declaration[..open].split('<').next().unwrap_or_default();
        if !name.is_empty()
            && name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
        {
            methods.insert(name.to_string());
        }
    }
    Some(methods.into_iter().collect())
}

#[test]
fn extracted_client_methods_follow_module_declarations_and_removals() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    std::fs::write(
        root.join("lib.rs"),
        "mod client;\nmod structured;\n#[cfg(test)]\nmod test_support;\n",
    )
    .unwrap();
    let client = "#[path = \"client_controls.rs\"]\nmod controls;\nimpl JcodeClient {\n    pub fn connect() {}\n}\n";
    let controls = "impl JcodeClient {\n    pub fn advisor() {}\n}\n";
    std::fs::write(root.join("client.rs"), client).unwrap();
    std::fs::write(root.join("client_controls.rs"), controls).unwrap();
    std::fs::write(
        root.join("structured.rs"),
        "impl JcodeClient {\n    pub fn run_structured() {}\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("orphan.rs"),
        "impl JcodeClient {\n    pub fn orphan() {}\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("test_support.rs"),
        "impl JcodeClient {\n    pub fn test_only() {}\n}\n",
    )
    .unwrap();
    assert_eq!(
        rust_public_methods_in(root),
        ["advisor", "connect", "run_structured"]
    );

    std::fs::write(
        root.join("client_controls.rs"),
        controls.replace("pub fn advisor", "fn advisor"),
    )
    .unwrap();
    assert_eq!(rust_public_methods_in(root), ["connect", "run_structured"]);

    std::fs::write(root.join("client_controls.rs"), controls).unwrap();
    std::fs::write(
        root.join("client.rs"),
        client
            .replace("mod controls;\n", "")
            .replace("#[path = \"client_controls.rs\"]\n", ""),
    )
    .unwrap();
    assert_eq!(rust_public_methods_in(root), ["connect", "run_structured"]);
}
