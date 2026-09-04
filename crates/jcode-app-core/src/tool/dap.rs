use super::{Tool, ToolContext, ToolOutput};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use jcode_dap::{
    DebugAdapterConfig, DebugBreakpointId, DebugBreakpointMutation, DebugContinueRequest,
    DebugEvaluateContext, DebugEvaluateOutcome, DebugEvaluateRequest, DebugEvaluateTarget,
    DebugExecutionRevision, DebugLaunchRequest, DebugOutputCursor, DebugOwnedAttachRequest,
    DebugPauseRequest, DebugRemoveBreakpointRequest, DebugScopesRequest, DebugSessionId,
    DebugSessionManager, DebugSetBreakpointRequest, DebugSourceBreakpoint, DebugStackFrameHandle,
    DebugStackTraceRequest, DebugStepRequest, DebugSteppingGranularity, DebugThreadId,
    DebugVariableFilter, DebugVariableHandle, DebugVariablesRequest, DebugWorkspaceKey,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

mod output;

const MAX_OUTPUT_CHARS: usize = 24_000;
const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 200;

#[derive(Clone)]
pub(crate) struct DapService {
    manager: Arc<DebugSessionManager>,
    tokens: Arc<Mutex<TokenBroker>>,
    max_output_bytes: usize,
    allow_evaluate: bool,
    adapters: Arc<BTreeMap<String, jcode_dap::DapAdapterConfig>>,
}

impl DapService {
    pub(crate) fn from_config(config: &jcode_dap::DapConfig) -> Result<Self> {
        let issues = config.validation_issues();
        if !issues.is_empty() {
            bail!("invalid DAP configuration: {}", issues.join("; "));
        }
        let mut manager_config = jcode_dap::DebugSessionManagerConfig::default();
        manager_config.output_max_bytes = config.max_output_bytes;
        let manager = Arc::new(DebugSessionManager::new(manager_config)?);
        Ok(Self {
            manager,
            tokens: Arc::new(Mutex::new(TokenBroker::new(
                config.max_opaque_handles_per_owner,
            ))),
            max_output_bytes: config.max_output_bytes,
            allow_evaluate: config.allow_evaluate,
            adapters: Arc::new(config.adapters.clone()),
        })
    }

    pub(crate) fn tool(&self) -> DapTool {
        DapTool {
            service: self.clone(),
        }
    }

    pub(crate) async fn cleanup_owner(&self, owner: &str) {
        self.tokens
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cleanup_owner(owner);
        let report = self
            .manager
            .cleanup_owner(owner, jcode_dap::OwnerCleanupCause::Disconnected)
            .await;
        for failure in report.failures {
            crate::logging::warn(&format!(
                "DAP owner cleanup failed for session {}: {}",
                failure.session_id, failure.message
            ));
        }
    }

    pub(crate) async fn shutdown_all(&self) {
        self.tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        let report = self.manager.shutdown_all().await;
        for failure in report.failures {
            crate::logging::warn(&format!(
                "DAP shutdown cleanup failed for session {}: {}",
                failure.session_id, failure.message
            ));
        }
    }
}

struct TokenBroker {
    max_per_owner: usize,
    order: VecDeque<TokenKey>,
    current_revisions: HashMap<String, HashMap<DebugSessionId, DebugExecutionRevision>>,
    sessions: HashMap<String, Owned<DebugSessionId>>,
    breakpoints: HashMap<String, Owned<DebugBreakpointId>>,
    frames: HashMap<String, Owned<DebugStackFrameHandle>>,
    variables: HashMap<String, Owned<DebugVariableHandle>>,
    cursors: HashMap<String, Owned<DebugOutputCursor>>,
    revisions: HashMap<String, Owned<DebugExecutionRevision>>,
}

struct Owned<T> {
    owner: String,
    session: Option<DebugSessionId>,
    revision: Option<DebugExecutionRevision>,
    value: T,
}

#[derive(Clone, Copy)]
enum TokenKind {
    Session,
    Breakpoint,
    Frame,
    Variable,
    Cursor,
    Revision,
}

struct TokenKey {
    kind: TokenKind,
    token: String,
}

impl TokenBroker {
    fn new(max_per_owner: usize) -> Self {
        Self {
            max_per_owner,
            order: VecDeque::new(),
            current_revisions: HashMap::new(),
            sessions: HashMap::new(),
            breakpoints: HashMap::new(),
            frames: HashMap::new(),
            variables: HashMap::new(),
            cursors: HashMap::new(),
            revisions: HashMap::new(),
        }
    }
    fn token(prefix: &str) -> String {
        format!("{prefix}_{}", Uuid::new_v4().simple())
    }
    fn owner_count(&self, owner: &str) -> usize {
        self.sessions.values().filter(|v| v.owner == owner).count()
            + self
                .breakpoints
                .values()
                .filter(|v| v.owner == owner)
                .count()
            + self.frames.values().filter(|v| v.owner == owner).count()
            + self.variables.values().filter(|v| v.owner == owner).count()
            + self.cursors.values().filter(|v| v.owner == owner).count()
            + self.revisions.values().filter(|v| v.owner == owner).count()
    }

    fn token_owner(&self, key: &TokenKey) -> Option<&str> {
        match key.kind {
            TokenKind::Session => self
                .sessions
                .get(&key.token)
                .map(|entry| entry.owner.as_str()),
            TokenKind::Breakpoint => self
                .breakpoints
                .get(&key.token)
                .map(|entry| entry.owner.as_str()),
            TokenKind::Frame => self
                .frames
                .get(&key.token)
                .map(|entry| entry.owner.as_str()),
            TokenKind::Variable => self
                .variables
                .get(&key.token)
                .map(|entry| entry.owner.as_str()),
            TokenKind::Cursor => self
                .cursors
                .get(&key.token)
                .map(|entry| entry.owner.as_str()),
            TokenKind::Revision => self
                .revisions
                .get(&key.token)
                .map(|entry| entry.owner.as_str()),
        }
    }

    fn remove_token(&mut self, key: &TokenKey) {
        match key.kind {
            TokenKind::Session => {
                self.sessions.remove(&key.token);
            }
            TokenKind::Breakpoint => {
                self.breakpoints.remove(&key.token);
            }
            TokenKind::Frame => {
                self.frames.remove(&key.token);
            }
            TokenKind::Variable => {
                self.variables.remove(&key.token);
            }
            TokenKind::Cursor => {
                self.cursors.remove(&key.token);
            }
            TokenKind::Revision => {
                self.revisions.remove(&key.token);
            }
        }
    }

    fn record(&mut self, kind: TokenKind, token: String) {
        self.order.push_back(TokenKey { kind, token });
    }

    fn compact_order(&mut self) {
        let sessions = &self.sessions;
        let breakpoints = &self.breakpoints;
        let frames = &self.frames;
        let variables = &self.variables;
        let cursors = &self.cursors;
        let revisions = &self.revisions;
        self.order.retain(|key| match key.kind {
            TokenKind::Session => sessions.contains_key(&key.token),
            TokenKind::Breakpoint => breakpoints.contains_key(&key.token),
            TokenKind::Frame => frames.contains_key(&key.token),
            TokenKind::Variable => variables.contains_key(&key.token),
            TokenKind::Cursor => cursors.contains_key(&key.token),
            TokenKind::Revision => revisions.contains_key(&key.token),
        });
    }

    fn ensure_capacity(&mut self, owner: &str) {
        self.compact_order();
        while self.owner_count(owner) >= self.max_per_owner {
            let Some(index) = self
                .order
                .iter()
                .position(|key| self.token_owner(key) == Some(owner))
            else {
                break;
            };
            if let Some(key) = self.order.remove(index) {
                self.remove_token(&key);
            }
        }
    }
    fn reserve_capacity(
        &mut self,
        owner: &str,
        count: usize,
        preserve_sessions: bool,
    ) -> Result<()> {
        if count > self.max_per_owner {
            bail!(
                "DAP response requires {count} opaque handles but the per-owner capacity is {}",
                self.max_per_owner
            );
        }
        self.compact_order();
        while self.owner_count(owner).saturating_add(count) > self.max_per_owner {
            let Some(index) = self
                .order
                .iter()
                .position(|key| {
                    self.token_owner(key) == Some(owner)
                        && (!preserve_sessions || !matches!(key.kind, TokenKind::Session))
                })
            else {
                bail!("unable to reserve DAP opaque-handle capacity");
            };
            if let Some(key) = self.order.remove(index) {
                self.remove_token(&key);
            }
        }
        Ok(())
    }
    fn put_session(&mut self, owner: &str, value: DebugSessionId) -> String {
        if let Some((token, _)) = self
            .sessions
            .iter()
            .find(|(_, entry)| entry.owner == owner && entry.value == value)
        {
            return token.clone();
        }
        self.ensure_capacity(owner);
        let t = Self::token("ds");
        self.sessions.insert(
            t.clone(),
            Owned {
                owner: owner.into(),
                session: None,
                revision: None,
                value,
            },
        );
        self.record(TokenKind::Session, t.clone());
        t
    }
    fn put_breakpoint(
        &mut self,
        owner: &str,
        session: DebugSessionId,
        value: DebugBreakpointId,
    ) -> String {
        if let Some((token, _)) = self.breakpoints.iter().find(|(_, entry)| {
            entry.owner == owner && entry.session == Some(session) && entry.value == value
        }) {
            return token.clone();
        }
        self.ensure_capacity(owner);
        let t = Self::token("db");
        self.breakpoints.insert(
            t.clone(),
            Owned {
                owner: owner.into(),
                session: Some(session),
                revision: None,
                value,
            },
        );
        self.record(TokenKind::Breakpoint, t.clone());
        t
    }
    fn put_frame(
        &mut self,
        owner: &str,
        session: DebugSessionId,
        revision: DebugExecutionRevision,
        value: DebugStackFrameHandle,
    ) -> String {
        self.ensure_capacity(owner);
        let t = Self::token("df");
        self.frames.insert(
            t.clone(),
            Owned {
                owner: owner.into(),
                session: Some(session),
                revision: Some(revision),
                value,
            },
        );
        self.record(TokenKind::Frame, t.clone());
        t
    }
    fn put_variable(
        &mut self,
        owner: &str,
        session: DebugSessionId,
        revision: DebugExecutionRevision,
        value: DebugVariableHandle,
    ) -> String {
        self.ensure_capacity(owner);
        let t = Self::token("dv");
        self.variables.insert(
            t.clone(),
            Owned {
                owner: owner.into(),
                session: Some(session),
                revision: Some(revision),
                value,
            },
        );
        self.record(TokenKind::Variable, t.clone());
        t
    }
    fn put_cursor(
        &mut self,
        owner: &str,
        session: DebugSessionId,
        value: DebugOutputCursor,
    ) -> String {
        self.ensure_capacity(owner);
        let t = Self::token("do");
        self.cursors.insert(
            t.clone(),
            Owned {
                owner: owner.into(),
                session: Some(session),
                revision: None,
                value,
            },
        );
        self.record(TokenKind::Cursor, t.clone());
        t
    }
    fn put_revision(
        &mut self,
        owner: &str,
        session: DebugSessionId,
        value: DebugExecutionRevision,
    ) -> String {
        self.advance_revision(owner, session, value);
        self.ensure_capacity(owner);
        let t = Self::token("dr");
        self.revisions.insert(
            t.clone(),
            Owned {
                owner: owner.into(),
                session: Some(session),
                revision: Some(value),
                value,
            },
        );
        self.record(TokenKind::Revision, t.clone());
        t
    }

    fn advance_revision(
        &mut self,
        owner: &str,
        session: DebugSessionId,
        revision: DebugExecutionRevision,
    ) {
        let previous = self
            .current_revisions
            .entry(owner.to_owned())
            .or_default()
            .insert(session, revision);
        if previous.is_some_and(|previous| previous != revision) {
            self.frames.retain(|_, entry| {
                entry.owner != owner
                    || entry.session != Some(session)
                    || entry.revision == Some(revision)
            });
            self.variables.retain(|_, entry| {
                entry.owner != owner
                    || entry.session != Some(session)
                    || entry.revision == Some(revision)
            });
            self.revisions.retain(|_, entry| {
                entry.owner != owner
                    || entry.session != Some(session)
                    || entry.revision == Some(revision)
            });
            self.compact_order();
        }
    }
    fn get<T: Clone>(
        map: &HashMap<String, Owned<T>>,
        owner: &str,
        session: Option<DebugSessionId>,
        token: &str,
        kind: &str,
    ) -> Result<T> {
        let entry = map
            .get(token)
            .ok_or_else(|| anyhow!("unknown or expired {kind} token"))?;
        if entry.owner != owner {
            bail!("{kind} token is not owned by this session");
        }
        if session.is_some() && entry.session != session {
            bail!("{kind} token belongs to a different debug session");
        }
        Ok(entry.value.clone())
    }
    fn session(&self, o: &str, t: &str) -> Result<DebugSessionId> {
        Self::get(&self.sessions, o, None, t, "session")
    }
    fn breakpoint(&self, o: &str, s: DebugSessionId, t: &str) -> Result<DebugBreakpointId> {
        Self::get(&self.breakpoints, o, Some(s), t, "breakpoint")
    }
    fn frame(&self, o: &str, s: DebugSessionId, t: &str) -> Result<DebugStackFrameHandle> {
        self.get_revision_scoped(&self.frames, o, s, t, "frame")
    }
    fn variable(&self, o: &str, s: DebugSessionId, t: &str) -> Result<DebugVariableHandle> {
        self.get_revision_scoped(&self.variables, o, s, t, "variables")
    }
    fn revision(&self, o: &str, s: DebugSessionId, t: &str) -> Result<DebugExecutionRevision> {
        Self::get(&self.revisions, o, Some(s), t, "execution revision")
    }
    fn cursor(&self, o: &str, s: DebugSessionId, t: &str) -> Result<DebugOutputCursor> {
        Self::get(&self.cursors, o, Some(s), t, "output cursor")
    }
    fn get_revision_scoped<T: Clone>(
        &self,
        map: &HashMap<String, Owned<T>>,
        owner: &str,
        session: DebugSessionId,
        token: &str,
        kind: &str,
    ) -> Result<T> {
        let entry = map
            .get(token)
            .ok_or_else(|| anyhow!("unknown or expired {kind} token"))?;
        if entry.owner != owner {
            bail!("{kind} token is not owned by this session");
        }
        if entry.session != Some(session) {
            bail!("{kind} token belongs to a different debug session");
        }
        let current = self
            .current_revisions
            .get(owner)
            .and_then(|revisions| revisions.get(&session));
        if entry.revision.as_ref() != current {
            bail!("stale {kind} token; refresh debugger state");
        }
        Ok(entry.value.clone())
    }
    fn remove_breakpoint_token(&mut self, token: &str) {
        self.breakpoints.remove(token);
    }
    fn cleanup_owner(&mut self, owner: &str) {
        self.sessions.retain(|_, v| v.owner != owner);
        self.breakpoints.retain(|_, v| v.owner != owner);
        self.frames.retain(|_, v| v.owner != owner);
        self.variables.retain(|_, v| v.owner != owner);
        self.cursors.retain(|_, v| v.owner != owner);
        self.revisions.retain(|_, v| v.owner != owner);
        self.current_revisions.remove(owner);
    }
    fn clear(&mut self) {
        self.sessions.clear();
        self.breakpoints.clear();
        self.frames.clear();
        self.variables.clear();
        self.cursors.clear();
        self.revisions.clear();
        self.order.clear();
        self.current_revisions.clear();
    }
    fn cleanup_session(&mut self, owner: &str, session: DebugSessionId) {
        self.sessions
            .retain(|_, value| value.owner != owner || value.value != session);
        self.breakpoints
            .retain(|_, v| v.owner != owner || v.session != Some(session));
        self.frames
            .retain(|_, v| v.owner != owner || v.session != Some(session));
        self.variables
            .retain(|_, v| v.owner != owner || v.session != Some(session));
        self.cursors
            .retain(|_, v| v.owner != owner || v.session != Some(session));
        self.revisions
            .retain(|_, v| v.owner != owner || v.session != Some(session));
        if let Some(revisions) = self.current_revisions.get_mut(owner) {
            revisions.remove(&session);
            if revisions.is_empty() {
                self.current_revisions.remove(owner);
            }
        }
    }
}

fn adapter_path(
    adapters: &BTreeMap<String, jcode_dap::DapAdapterConfig>,
    adapter_id: &str,
) -> Result<PathBuf> {
    let configured = adapters
        .get(adapter_id)
        .ok_or_else(|| anyhow!("unknown DAP adapter id: {adapter_id}"))?;
    match configured.kind {
        jcode_dap::DapAdapterKind::LldbDap => {}
    }
    let command = configured.command.trim();
    let path = PathBuf::from(command);
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(command))
                .find(|candidate| candidate.is_file())
        })
        .ok_or_else(|| anyhow!("DAP adapter command not found on PATH: {command}"))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    action: String,
    adapter: Option<String>,
    session: Option<String>,
    program: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<String>,
    #[serde(default)]
    stop_on_entry: bool,
    source: Option<String>,
    line: Option<u64>,
    column: Option<u64>,
    condition: Option<String>,
    hit_condition: Option<String>,
    log_message: Option<String>,
    breakpoint: Option<String>,
    thread_id: Option<i32>,
    frame: Option<String>,
    variables: Option<String>,
    cursor: Option<String>,
    start: Option<u32>,
    count: Option<u32>,
    filter: Option<String>,
    granularity: Option<String>,
    context: Option<String>,
    expression: Option<String>,
    execution_revision: Option<String>,
    #[serde(default)]
    allow_side_effects: bool,
    intent: String,
}

pub(crate) struct DapTool {
    service: DapService,
}

#[async_trait]
impl Tool for DapTool {
    fn name(&self) -> &str {
        "dap"
    }
    fn description(&self) -> &str {
        "Control an owned Debug Adapter Protocol session. Available only when DAP is enabled."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type":"object", "additionalProperties":false,
            "properties":{
                "action":{"type":"string","enum":["launch","attach","set_breakpoint","remove_breakpoint","continue","pause","step_over","step_in","step_out","threads","stack_trace","scopes","variables","evaluate","output","sessions","terminate"]},
                "session":{"type":"string"}, "adapter":{"type":"string"}, "program":{"type":"string"}, "args":{"type":"array","items":{"type":"string"}}, "cwd":{"type":"string"}, "stop_on_entry":{"type":"boolean"},
                "source":{"type":"string"}, "line":{"type":"integer","minimum":1}, "column":{"type":"integer","minimum":1}, "condition":{"type":"string"}, "hit_condition":{"type":"string"}, "log_message":{"type":"string"}, "breakpoint":{"type":"string"},
                "thread_id":{"type":"integer"}, "frame":{"type":"string"}, "variables":{"type":"string"}, "cursor":{"type":"string"}, "start":{"type":"integer","minimum":0}, "count":{"type":"integer","minimum":1,"maximum":200}, "filter":{"type":"string","enum":["named","indexed"]}, "granularity":{"type":"string","enum":["statement","line","instruction"]}, "context":{"type":"string","enum":["unspecified","watch","repl","hover","clipboard","variables"]},
                "expression":{"type":"string"}, "execution_revision":{"type":"string"}, "allow_side_effects":{"type":"boolean"}, "intent":super::intent_schema_property()
            }, "required":["action","intent"]
        })
    }
    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let p: Input = serde_json::from_value(input).context("invalid DAP tool input")?;
        let owner = ctx.session_id.as_str();
        let root = ctx
            .working_dir
            .as_deref()
            .ok_or_else(|| anyhow!("DAP requires the session working directory"))?;
        let workspace = DebugWorkspaceKey::new(root, workspace_identity(root)?)?;
        let value = self.execute_action(&p, owner, workspace).await?;
        let envelope = json!({"protocol":"jcode.dap.v1","action":p.action,"result":value});
        let text = bounded_pretty(&envelope, self.service.max_output_bytes);
        Ok(ToolOutput::new(text).with_title(format!("dap: {}", p.action)).with_metadata(json!({"protocol":"jcode.dap.v1","action":p.action,"execution_class":output::execution_class(&p.action),"intent":p.intent})))
    }
}

impl DapTool {
    fn page_count(&self, requested: Option<u32>) -> Result<u32> {
        let count = requested.unwrap_or(DEFAULT_PAGE_SIZE);
        if !(1..=MAX_PAGE_SIZE).contains(&count) {
            bail!("count must be between 1 and {MAX_PAGE_SIZE}");
        }
        Ok(count)
    }

    async fn execute_action(
        &self,
        p: &Input,
        owner: &str,
        workspace: DebugWorkspaceKey,
    ) -> Result<Value> {
        let manager = &self.service.manager;
        match p.action.as_str() {
            "launch" | "attach" => {
                let program = p
                    .program
                    .as_deref()
                    .ok_or_else(|| anyhow!("program is required"))?;
                let adapter_id = p.adapter.as_deref().unwrap_or("lldb-dap");
                let adapter = DebugAdapterConfig::lldb_dap(adapter_path(
                    self.service.adapters.as_ref(),
                    adapter_id,
                )?)?;
                let snapshot = if p.action == "launch" {
                    let mut r = DebugLaunchRequest::new(program)
                        .with_args(p.args.clone())
                        .with_stop_on_entry(p.stop_on_entry);
                    if let Some(c) = &p.cwd {
                        r = r.with_cwd(c);
                    }
                    manager.launch(owner, workspace, &adapter, r).await?
                } else {
                    let mut r = DebugOwnedAttachRequest::new(program).with_args(p.args.clone());
                    if let Some(c) = &p.cwd {
                        r = r.with_cwd(c);
                    }
                    manager
                        .spawn_and_attach(owner, workspace, &adapter, r)
                        .await?
                };
                let token = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner())
                    .put_session(owner, snapshot.id);
                Ok(
                    json!({"session":token,"state":output::session_state(snapshot.state.kind()),"adapter":snapshot.adapter_id}),
                )
            }
            "sessions" => {
                let snapshots = manager.sessions(owner);
                let mut b = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner());
                b.reserve_capacity(owner, snapshots.len(), false)?;
                Ok(Value::Array(snapshots.into_iter().map(|s|json!({"session":b.put_session(owner,s.id),"state":output::session_state(s.state.kind()),"adapter":s.adapter_id})).collect()))
            }
            action => {
                let st = p
                    .session
                    .as_deref()
                    .ok_or_else(|| anyhow!("session is required for {action}"))?;
                let id = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner())
                    .session(owner, st)?;
                self.execute_session_action(action, p, owner, id).await
            }
        }
    }

    async fn execute_session_action(
        &self,
        a: &str,
        p: &Input,
        owner: &str,
        id: DebugSessionId,
    ) -> Result<Value> {
        let m = &self.service.manager;
        let thread = || p.thread_id.map(DebugThreadId::new);
        match a {
            "terminate" => {
                let s = m.terminate(owner, id).await?;
                self.service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner())
                    .cleanup_session(owner, id);
                Ok(json!({"state":output::session_state(s.state.kind())}))
            }
            "threads" => {
                let r = m.threads(owner, id).await?;
                let mut b = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner());
                b.reserve_capacity(owner, 1, true)?;
                Ok(
                    json!({"execution_revision":b.put_revision(owner, id,r.execution_revision),"state":output::session_state(r.state),"threads":r.threads.into_iter().map(|t|json!({"id":t.id.get(),"name":t.name})).collect::<Vec<_>>() }),
                )
            }
            "continue" | "pause" | "step_over" | "step_in" | "step_out" => {
                let rev = p
                    .execution_revision
                    .as_deref()
                    .map(|token| {
                        self.service
                            .tokens
                            .lock()
                            .unwrap_or_else(|x| x.into_inner())
                            .revision(owner, id, token)
                    })
                    .transpose()?;
                let r = match a {
                    "continue" => {
                        let mut q = DebugContinueRequest::default();
                        q.thread_id = thread();
                        q.expected_execution_revision = rev;
                        m.continue_execution(owner, id, q).await?
                    }
                    "pause" => {
                        let mut q = DebugPauseRequest::default();
                        q.thread_id = thread();
                        q.expected_execution_revision = rev;
                        m.pause(owner, id, q).await?
                    }
                    _ => {
                        let mut q = DebugStepRequest::default();
                        q.thread_id = thread();
                        q.expected_execution_revision = rev;
                        q.granularity = match p.granularity.as_deref() {
                            None | Some("statement") => DebugSteppingGranularity::Statement,
                            Some("line") => DebugSteppingGranularity::Line,
                            Some("instruction") => DebugSteppingGranularity::Instruction,
                            Some(other) => bail!("unsupported step granularity: {other}"),
                        };
                        match a {
                            "step_over" => m.step_over(owner, id, q).await?,
                            "step_in" => m.step_in(owner, id, q).await?,
                            _ => m.step_out(owner, id, q).await?,
                        }
                    }
                };
                let mut b = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner());
                b.reserve_capacity(owner, 1, true)?;
                let revision = b.put_revision(owner, id, r.execution_revision);
                Ok(
                    json!({"operation":output::control_operation(r.operation),"thread_id":r.thread_id.get(),"execution_revision":revision,"state":output::session_state(r.state.kind())}),
                )
            }
            "set_breakpoint" => {
                let source = p
                    .source
                    .as_deref()
                    .ok_or_else(|| anyhow!("source is required"))?;
                let mut bp =
                    DebugSourceBreakpoint::new(p.line.ok_or_else(|| anyhow!("line is required"))?);
                if let Some(v) = p.column {
                    bp = bp.with_column(v)
                }
                if let Some(v) = &p.condition {
                    bp = bp.with_condition(v)
                }
                if let Some(v) = &p.hit_condition {
                    bp = bp.with_hit_condition(v)
                }
                if let Some(v) = &p.log_message {
                    bp = bp.with_log_message(v)
                }
                let r = m
                    .set_breakpoint(owner, id, DebugSetBreakpointRequest::new(source, bp))
                    .await?;
                let bid = match r.mutation {
                    DebugBreakpointMutation::Created { breakpoint_id }
                    | DebugBreakpointMutation::Existing { breakpoint_id } => breakpoint_id,
                    _ => bail!("unexpected breakpoint mutation"),
                };
                let mut broker = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner());
                broker.reserve_capacity(owner, 1, true)?;
                let token = broker.put_breakpoint(owner, id, bid);
                Ok(
                    json!({"breakpoint":token,"verified":r.source.breakpoints.iter().find(|b|b.id==bid).map(|b|b.verified)}),
                )
            }
            "remove_breakpoint" => {
                let token = p
                    .breakpoint
                    .as_deref()
                    .ok_or_else(|| anyhow!("breakpoint is required"))?;
                let bid = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner())
                    .breakpoint(owner, id, token)?;
                m.remove_breakpoint(owner, id, DebugRemoveBreakpointRequest::new(bid))
                    .await?;
                self.service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner())
                    .remove_breakpoint_token(token);
                Ok(json!({"removed":token}))
            }
            "stack_trace" => {
                let mut q = DebugStackTraceRequest::new(self.page_count(p.count)?);
                q.thread_id = thread();
                q.start_frame = p.start.unwrap_or(0);
                let r = m.stack_trace(owner, id, q).await?;
                let mut b = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner());
                let execution_revision = r.execution_revision;
                b.reserve_capacity(owner, r.frames.len().saturating_add(1), true)?;
                let revision = b.put_revision(owner, id, execution_revision);
                let frames = r
                    .frames
                    .into_iter()
                    .map(|f| {
                        json!({
                            "frame": b.put_frame(owner, id, execution_revision, f.handle),
                            "name": f.name,
                            "source": f.location.source.and_then(|source| source.path),
                            "line": f.location.line,
                            "column": f.location.column
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(json!({
                    "thread_id": r.thread_id.get(),
                    "execution_revision": revision,
                    "next_start": r.next_start_frame,
                    "frames": frames
                }))
            }
            "scopes" => {
                let frame = if let Some(frame_token) = p.frame.as_deref() {
                    self.service
                        .tokens
                        .lock()
                        .unwrap_or_else(|x| x.into_inner())
                        .frame(owner, id, frame_token)?
                } else {
                    let mut request = DebugStackTraceRequest::new(1);
                    request.thread_id = thread();
                    let stack = m.stack_trace(owner, id, request).await?;
                    stack
                        .frames
                        .into_iter()
                        .next()
                        .map(|frame| frame.handle)
                        .ok_or_else(|| {
                            anyhow!("no current stack frame is available; supply a frame token")
                        })?
                };
                let expected_execution_revision = p
                    .execution_revision
                    .as_deref()
                    .map(|token| {
                        self.service
                            .tokens
                            .lock()
                            .unwrap_or_else(|x| x.into_inner())
                            .revision(owner, id, token)
                    })
                    .transpose()?;
                let r = m
                    .scopes(
                        owner,
                        id,
                        DebugScopesRequest {
                            frame,
                            expected_execution_revision,
                        },
                    )
                    .await?;
                let mut b = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner());
                let execution_revision = r.execution_revision;
                b.reserve_capacity(owner, r.scopes.len().saturating_add(1), true)?;
                let revision = b.put_revision(owner, id, execution_revision);
                let scopes = r
                    .scopes
                    .into_iter()
                    .map(|scope| {
                        json!({
                            "name": scope.name,
                            "variables": b.put_variable(
                                owner,
                                id,
                                execution_revision,
                                scope.variables,
                            ),
                            "expensive": scope.expensive,
                            "named": scope.named_variables,
                            "indexed": scope.indexed_variables
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(json!({"execution_revision": revision, "scopes": scopes}))
            }
            "variables" => {
                let vt = p
                    .variables
                    .as_deref()
                    .ok_or_else(|| anyhow!("variables is required"))?;
                let vars = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner())
                    .variable(owner, id, vt)?;
                let filter = match p.filter.as_deref() {
                    Some("named") => Some(DebugVariableFilter::Named),
                    Some("indexed") => Some(DebugVariableFilter::Indexed),
                    None => None,
                    _ => bail!("invalid variable filter"),
                };
                let r = m
                    .variables(
                        owner,
                        id,
                        DebugVariablesRequest {
                            variables: vars,
                            filter,
                            start: p.start.unwrap_or(0),
                            count: self.page_count(p.count)?,
                            expected_execution_revision: p
                                .execution_revision
                                .as_deref()
                                .map(|token| {
                                    self.service
                                        .tokens
                                        .lock()
                                        .unwrap_or_else(|x| x.into_inner())
                                        .revision(owner, id, token)
                                })
                                .transpose()?,
                        },
                    )
                    .await?;
                let mut b = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner());
                let execution_revision = r.execution_revision;
                let handle_count = r
                    .variables
                    .iter()
                    .filter(|variable| variable.variables.is_expandable())
                    .count()
                    .saturating_add(1);
                b.reserve_capacity(owner, handle_count, true)?;
                let revision = b.put_revision(owner, id, execution_revision);
                let variables = r
                    .variables
                    .into_iter()
                    .map(|variable| {
                        let child = variable.variables.is_expandable().then(|| {
                            b.put_variable(owner, id, execution_revision, variable.variables)
                        });
                        json!({
                            "name": variable.name,
                            "value": variable.value.text,
                            "omitted_bytes": variable.value.omitted_bytes,
                            "type": variable.type_name,
                            "variables": child
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(json!({
                    "execution_revision": revision,
                    "next_start": r.next_start,
                    "variables": variables
                }))
            }
            "evaluate" => {
                if !self.service.allow_evaluate {
                    bail!("DAP evaluate is disabled by Config.dap.allow_evaluate")
                }
                if !p.allow_side_effects {
                    bail!("evaluate requires explicit allow_side_effects: true")
                }
                let expression = p
                    .expression
                    .clone()
                    .ok_or_else(|| anyhow!("expression is required"))?;
                let revision_token = p
                    .execution_revision
                    .as_deref()
                    .ok_or_else(|| anyhow!("execution_revision is required"))?;
                let revision = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner())
                    .revision(owner, id, revision_token)?;
                let target = if let Some(t) = &p.frame {
                    DebugEvaluateTarget::Frame(
                        self.service
                            .tokens
                            .lock()
                            .unwrap_or_else(|x| x.into_inner())
                            .frame(owner, id, t)?,
                    )
                } else {
                    DebugEvaluateTarget::Global
                };
                let r = m
                    .evaluate(
                        owner,
                        id,
                        DebugEvaluateRequest {
                            expression,
                            context: match p.context.as_deref() {
                                None | Some("unspecified") => DebugEvaluateContext::Unspecified,
                                Some("watch") => DebugEvaluateContext::Watch,
                                Some("repl") => DebugEvaluateContext::Repl,
                                Some("hover") => DebugEvaluateContext::Hover,
                                Some("clipboard") => DebugEvaluateContext::Clipboard,
                                Some("variables") => DebugEvaluateContext::Variables,
                                Some(other) => bail!("unsupported evaluate context: {other}"),
                            },
                            target,
                            expected_execution_revision: revision,
                        },
                    )
                    .await?;
                match r {
                    DebugEvaluateOutcome::Known(v) => {
                        let mut b = self
                            .service
                            .tokens
                            .lock()
                            .unwrap_or_else(|x| x.into_inner());
                        let execution_revision = v.execution_revision;
                        let handle_count = usize::from(v.variables.is_expandable()).saturating_add(1);
                        b.reserve_capacity(owner, handle_count, true)?;
                        let revision = b.put_revision(owner, id, execution_revision);
                        let variables = v
                            .variables
                            .is_expandable()
                            .then(|| b.put_variable(owner, id, execution_revision, v.variables));
                        Ok(json!({
                            "status": "known",
                            "execution_revision": revision,
                            "result": v.result,
                            "type": v.type_name,
                            "variables": variables
                        }))
                    }
                    DebugEvaluateOutcome::Unknown(v) => Ok(
                        json!({"status":"unknown","reason":output::evaluate_unknown_reason(v.reason)}),
                    ),
                    _ => Ok(json!({"status":"unknown","reason":"unsupported_outcome"})),
                }
            }
            "output" => {
                let after = match &p.cursor {
                    Some(t) => Some(
                        self.service
                            .tokens
                            .lock()
                            .unwrap_or_else(|x| x.into_inner())
                            .cursor(owner, id, t)?,
                    ),
                    None => None,
                };
                let page = m.output(owner, id, after, self.page_count(p.count)? as usize)?;
                let mut b = self
                    .service
                    .tokens
                    .lock()
                    .unwrap_or_else(|x| x.into_inner());
                b.reserve_capacity(owner, 1, true)?;
                Ok(
                    json!({"records":page.records.into_iter().map(|r|json!({"category":output::output_category(r.category),"output":r.output,"truncated_prefix_bytes":r.truncated_prefix_bytes})).collect::<Vec<_>>(),"cursor":b.put_cursor(owner, id,page.status.next_cursor),"retained_events":page.status.retained_events,"retained_bytes":page.status.retained_bytes,"requested_history_was_evicted":page.requested_history_was_evicted}),
                )
            }
            _ => bail!("unsupported DAP action: {a}"),
        }
    }
}

fn workspace_identity(root: &Path) -> Result<String> {
    Ok(root.canonicalize()?.to_string_lossy().into_owned())
}
fn bounded_pretty(value: &Value, configured_max: usize) -> String {
    let limit = configured_max.min(MAX_OUTPUT_CHARS).max(1024);
    let serialized = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
    if serialized.len() <= limit {
        return serialized;
    }
    serde_json::to_string_pretty(&json!({
        "protocol": "jcode.dap.v1",
        "action": value.get("action").and_then(Value::as_str).unwrap_or("unknown"),
        "result": {
            "truncated": true,
            "original_bytes": serialized.len(),
            "message": "DAP result exceeded the configured tool-output bound; request a smaller page"
        }
    }))
    .unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests;
