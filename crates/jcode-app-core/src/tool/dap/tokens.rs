use anyhow::{Result, anyhow, bail};
use jcode_dap::{
    DebugBreakpointId, DebugExecutionRevision, DebugOutputCursor, DebugSessionId,
    DebugStackFrameHandle, DebugStepInTargetHandle, DebugVariableHandle,
};
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;

pub(super) struct TokenBroker {
    max_per_owner: usize,
    pub(super) order: VecDeque<TokenKey>,
    current_revisions: HashMap<String, HashMap<DebugSessionId, DebugExecutionRevision>>,
    sessions: HashMap<String, Owned<DebugSessionId>>,
    breakpoints: HashMap<String, Owned<DebugBreakpointId>>,
    pub(super) frames: HashMap<String, Owned<DebugStackFrameHandle>>,
    step_in_targets: HashMap<String, Owned<DebugStepInTargetHandle>>,
    variables: HashMap<String, Owned<DebugVariableHandle>>,
    cursors: HashMap<String, Owned<DebugOutputCursor>>,
    revisions: HashMap<String, Owned<DebugExecutionRevision>>,
}

pub(super) struct Owned<T> {
    owner: String,
    session: Option<DebugSessionId>,
    revision: Option<DebugExecutionRevision>,
    value: T,
}

#[derive(Clone, Copy)]
pub(super) enum TokenKind {
    Session,
    Breakpoint,
    Frame,
    StepInTarget,
    Variable,
    Cursor,
    Revision,
}

pub(super) struct TokenKey {
    kind: TokenKind,
    token: String,
}

impl TokenBroker {
    pub(super) fn new(max_per_owner: usize) -> Self {
        Self {
            max_per_owner,
            order: VecDeque::new(),
            current_revisions: HashMap::new(),
            sessions: HashMap::new(),
            breakpoints: HashMap::new(),
            frames: HashMap::new(),
            step_in_targets: HashMap::new(),
            variables: HashMap::new(),
            cursors: HashMap::new(),
            revisions: HashMap::new(),
        }
    }
    pub(super) fn token(prefix: &str) -> String {
        format!("{prefix}_{}", Uuid::new_v4().simple())
    }
    pub(super) fn owner_count(&self, owner: &str) -> usize {
        self.sessions.values().filter(|v| v.owner == owner).count()
            + self
                .breakpoints
                .values()
                .filter(|v| v.owner == owner)
                .count()
            + self.frames.values().filter(|v| v.owner == owner).count()
            + self
                .step_in_targets
                .values()
                .filter(|v| v.owner == owner)
                .count()
            + self.variables.values().filter(|v| v.owner == owner).count()
            + self.cursors.values().filter(|v| v.owner == owner).count()
            + self.revisions.values().filter(|v| v.owner == owner).count()
    }

    pub(super) fn token_owner(&self, key: &TokenKey) -> Option<&str> {
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
            TokenKind::StepInTarget => self
                .step_in_targets
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

    pub(super) fn remove_token(&mut self, key: &TokenKey) {
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
            TokenKind::StepInTarget => {
                self.step_in_targets.remove(&key.token);
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

    pub(super) fn record(&mut self, kind: TokenKind, token: String) {
        self.order.push_back(TokenKey { kind, token });
    }

    pub(super) fn compact_order(&mut self) {
        let sessions = &self.sessions;
        let breakpoints = &self.breakpoints;
        let frames = &self.frames;
        let step_in_targets = &self.step_in_targets;
        let variables = &self.variables;
        let cursors = &self.cursors;
        let revisions = &self.revisions;
        self.order.retain(|key| match key.kind {
            TokenKind::Session => sessions.contains_key(&key.token),
            TokenKind::Breakpoint => breakpoints.contains_key(&key.token),
            TokenKind::Frame => frames.contains_key(&key.token),
            TokenKind::StepInTarget => step_in_targets.contains_key(&key.token),
            TokenKind::Variable => variables.contains_key(&key.token),
            TokenKind::Cursor => cursors.contains_key(&key.token),
            TokenKind::Revision => revisions.contains_key(&key.token),
        });
    }

    pub(super) fn ensure_capacity(&mut self, owner: &str) {
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
    pub(super) fn reserve_capacity(
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
            let Some(index) = self.order.iter().position(|key| {
                self.token_owner(key) == Some(owner)
                    && (!preserve_sessions || !matches!(key.kind, TokenKind::Session))
            }) else {
                bail!("unable to reserve DAP opaque-handle capacity");
            };
            if let Some(key) = self.order.remove(index) {
                self.remove_token(&key);
            }
        }
        Ok(())
    }
    pub(super) fn put_session(&mut self, owner: &str, value: DebugSessionId) -> String {
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
    pub(super) fn put_breakpoint(
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
    pub(super) fn put_frame(
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
    pub(super) fn put_variable(
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
    pub(super) fn put_step_in_target(
        &mut self,
        owner: &str,
        session: DebugSessionId,
        revision: DebugExecutionRevision,
        value: DebugStepInTargetHandle,
    ) -> String {
        self.ensure_capacity(owner);
        let token = Self::token("dt");
        self.step_in_targets.insert(
            token.clone(),
            Owned {
                owner: owner.into(),
                session: Some(session),
                revision: Some(revision),
                value,
            },
        );
        self.record(TokenKind::StepInTarget, token.clone());
        token
    }
    pub(super) fn put_cursor(
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
    pub(super) fn put_revision(
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

    pub(super) fn advance_revision(
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
            self.step_in_targets.retain(|_, entry| {
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
    pub(super) fn get<T: Clone>(
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
    pub(super) fn session(&self, o: &str, t: &str) -> Result<DebugSessionId> {
        Self::get(&self.sessions, o, None, t, "session")
    }
    pub(super) fn breakpoint(
        &self,
        o: &str,
        s: DebugSessionId,
        t: &str,
    ) -> Result<DebugBreakpointId> {
        Self::get(&self.breakpoints, o, Some(s), t, "breakpoint")
    }
    pub(super) fn frame(
        &self,
        o: &str,
        s: DebugSessionId,
        t: &str,
    ) -> Result<DebugStackFrameHandle> {
        self.get_revision_scoped(&self.frames, o, s, t, "frame")
    }
    pub(super) fn variable(
        &self,
        o: &str,
        s: DebugSessionId,
        t: &str,
    ) -> Result<DebugVariableHandle> {
        self.get_revision_scoped(&self.variables, o, s, t, "variables")
    }
    pub(super) fn step_in_target(
        &self,
        owner: &str,
        session: DebugSessionId,
        token: &str,
    ) -> Result<DebugStepInTargetHandle> {
        self.get_revision_scoped(
            &self.step_in_targets,
            owner,
            session,
            token,
            "step-in target",
        )
    }
    pub(super) fn revision(
        &self,
        o: &str,
        s: DebugSessionId,
        t: &str,
    ) -> Result<DebugExecutionRevision> {
        Self::get(&self.revisions, o, Some(s), t, "execution revision")
    }
    pub(super) fn cursor(&self, o: &str, s: DebugSessionId, t: &str) -> Result<DebugOutputCursor> {
        Self::get(&self.cursors, o, Some(s), t, "output cursor")
    }
    pub(super) fn get_revision_scoped<T: Clone>(
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
    pub(super) fn remove_breakpoint_token(&mut self, token: &str) {
        self.breakpoints.remove(token);
    }
    pub(super) fn cleanup_owner(&mut self, owner: &str) {
        self.sessions.retain(|_, v| v.owner != owner);
        self.breakpoints.retain(|_, v| v.owner != owner);
        self.frames.retain(|_, v| v.owner != owner);
        self.step_in_targets.retain(|_, v| v.owner != owner);
        self.variables.retain(|_, v| v.owner != owner);
        self.cursors.retain(|_, v| v.owner != owner);
        self.revisions.retain(|_, v| v.owner != owner);
        self.current_revisions.remove(owner);
    }
    pub(super) fn clear(&mut self) {
        self.sessions.clear();
        self.breakpoints.clear();
        self.frames.clear();
        self.step_in_targets.clear();
        self.variables.clear();
        self.cursors.clear();
        self.revisions.clear();
        self.order.clear();
        self.current_revisions.clear();
    }
    pub(super) fn cleanup_session(&mut self, owner: &str, session: DebugSessionId) {
        self.sessions
            .retain(|_, value| value.owner != owner || value.value != session);
        self.breakpoints
            .retain(|_, v| v.owner != owner || v.session != Some(session));
        self.frames
            .retain(|_, v| v.owner != owner || v.session != Some(session));
        self.step_in_targets
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
