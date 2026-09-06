//! Api requests.

use super::*;

impl BridgeState {
    /// Translate one API request (raw JSON) into outbound actions.
    pub fn api_request_to_legacy(&mut self, request: &Value) -> Vec<Outbound> {
        let api_id = request["id"].as_u64().unwrap_or(0);
        let req = request["req"].as_str().unwrap_or("");

        // Stateful requests only mean something once this connection is
        // attached. Forwarding one before then is not merely useless: the
        // daemon answers "Client must Subscribe with a working_dir before
        // sending stateful requests" and *closes the connection*, so a client
        // that mistypes a session id loses every other session it was
        // streaming, and the SDK reports a bare EPIPE. Answer locally instead,
        // with the code that actually says what went wrong.
        // `pending_attach_id` means a subscribe is already on the wire, so the
        // daemon will have a session by the time this arrives: a client that
        // pipelines `create_session` and `send_message` without awaiting must
        // still work. Only a connection that never asked to attach is refused.
        if self.session_id.is_none()
            && self.pending_attach_id.is_none()
            && REQUIRES_ATTACH.contains(&req)
        {
            let requested = request["session_id"].as_str().unwrap_or("");
            return vec![Outbound::Reply(ServerFrame::reply(
                api_id,
                ApiEvent::Error {
                    code: ErrorCode::UnknownSession,
                    message: if requested.is_empty() {
                        format!(
                            "`{req}` needs an attached session; call create_session or attach_session first"
                        )
                    } else {
                        format!(
                            "not attached to session `{requested}`; call attach_session first (it is not attached, or does not exist)"
                        )
                    },
                },
            ))];
        }

        // A request naming a session other than the attached one would be
        // silently applied to the attached session, because the legacy
        // protocol has no session field: `clear` on a typo'd id would wipe the
        // wrong transcript. Refuse rather than destroy the wrong thing.
        if let Some(attached) = self.session_id.as_deref()
            && REQUIRES_ATTACH.contains(&req)
            && let Some(requested) = request["session_id"].as_str()
            && !requested.is_empty()
            && requested != attached
        {
            return vec![Outbound::Reply(ServerFrame::reply(
                api_id,
                ApiEvent::Error {
                    code: ErrorCode::UnknownSession,
                    message: format!(
                        "this connection is attached to `{attached}`, not `{requested}`; attach to it first or use another connection"
                    ),
                },
            ))];
        }

        match req {
            "advisor" => self.advisor_request_to_legacy(request, api_id),
            "archive_session" => {
                let session_id = request["session_id"].as_str().unwrap_or("");
                if Self::session_record_path(session_id).is_none_or(|path| !path.is_file()) {
                    return Self::error_reply(
                        api_id,
                        ErrorCode::UnknownSession,
                        "session does not exist",
                    );
                }
                let _write_guard = Self::state_write_guard();
                let mut archive = Self::load_archive_state();
                archive
                    .sessions
                    .insert(session_id.to_string(), Self::now_ms());
                match Self::save_archive_state(&archive) {
                    Ok(()) => vec![Outbound::Reply(ServerFrame::reply(api_id, ApiEvent::Ok))],
                    Err(message) => Self::error_reply(api_id, ErrorCode::Internal, &message),
                }
            }
            "restore_session" => {
                let session_id = request["session_id"].as_str().unwrap_or("");
                if Self::session_record_path(session_id).is_none_or(|path| !path.is_file()) {
                    return Self::error_reply(
                        api_id,
                        ErrorCode::UnknownSession,
                        "session does not exist",
                    );
                }
                let _write_guard = Self::state_write_guard();
                let mut archive = Self::load_archive_state();
                archive.sessions.remove(session_id);
                match Self::save_archive_state(&archive) {
                    Ok(()) => vec![Outbound::Reply(ServerFrame::reply(api_id, ApiEvent::Ok))],
                    Err(message) => Self::error_reply(api_id, ErrorCode::Internal, &message),
                }
            }
            "set_retention_policy" => {
                let days = match request.get("archive_after_days") {
                    None | Some(Value::Null) => None,
                    Some(value) => match value.as_u64() {
                        Some(days @ 1..=36_500) => Some(days as u32),
                        _ => {
                            return Self::error_reply(
                                api_id,
                                ErrorCode::InvalidRequest,
                                "archive_after_days must be 1..=36500, or null to disable",
                            );
                        }
                    },
                };
                let _write_guard = Self::state_write_guard();
                let mut archive = Self::load_archive_state();
                archive.archive_after_days = days;
                match Self::save_archive_state(&archive) {
                    Ok(()) => vec![Outbound::Reply(ServerFrame::reply(api_id, ApiEvent::Ok))],
                    Err(message) => Self::error_reply(api_id, ErrorCode::Internal, &message),
                }
            }
            "create_session" | "attach_session" => {
                let id = self.legacy_id();
                let state_id = self.legacy_id();
                let catalog_id = self.legacy_id();
                let requested_session = (req == "attach_session")
                    .then(|| request["session_id"].as_str().map(str::to_string))
                    .flatten();
                let working_dir = match request["working_dir"].as_str() {
                    Some(directory) => directory.to_string(),
                    None => match std::env::current_dir() {
                        Ok(directory) => directory.display().to_string(),
                        Err(error) => {
                            return Self::error_reply(
                                api_id,
                                ErrorCode::Internal,
                                &format!(
                                    "Could not resolve the session working directory: {error}"
                                ),
                            );
                        }
                    },
                };
                self.pending_attach_id = Some((state_id, api_id, requested_session));
                self.pending_model_probe = Some(catalog_id);
                let mut subscribe = json!({
                    "type": "subscribe",
                    "id": id,
                    "working_dir": working_dir,
                });
                if self.crash_on_disconnect {
                    subscribe["crash_on_disconnect"] = json!(true);
                }
                // Sessions rooted inside a jcode checkout are self-dev
                // sessions: the daemon only enables the self-dev tools and
                // prompt when the subscribe says so, and a client that opens
                // the repo without saying so gets an agent that cannot build
                // the very app it is running in.
                if Self::path_is_inside_jcode_repo(&working_dir) {
                    subscribe["selfdev"] = json!(true);
                }
                if req == "attach_session"
                    && let Some(target) = request["session_id"].as_str()
                {
                    subscribe["target_session_id"] = json!(target);
                }
                // The daemon assigns the session during subscribe but reports
                // the id via `state`, so chase the subscribe with get_state.
                // The model identity arrives the same way, via the catalog
                // reply, so ask for it now rather than making the client poll.
                vec![
                    Outbound::Legacy(subscribe),
                    Outbound::Legacy(json!({"type": "state", "id": state_id})),
                    Outbound::Legacy(json!({"type": "get_model_catalog", "id": catalog_id})),
                ]
            }
            "send_message" => {
                let id = self.legacy_id();
                let no_reply = request["no_reply"].as_bool().unwrap_or(false);
                if no_reply {
                    self.pending_no_reply_message_id = Some((id, api_id));
                } else {
                    self.pending_message_id = Some(id);
                }
                let mut message = json!({
                    "type": "message",
                    "id": id,
                    "content": request["content"].as_str().unwrap_or(""),
                });
                if no_reply {
                    message["no_reply"] = json!(true);
                }
                if let Some(images) = request["images"].as_array()
                    && !images.is_empty()
                {
                    message["images"] = json!(images);
                }
                vec![Outbound::Legacy(message)]
            }
            "fork_session" => {
                let id = self.legacy_id();
                self.pending_fork_id = Some((id, api_id));
                vec![Outbound::Legacy(json!({"type": "split", "id": id}))]
            }
            "cancel" => {
                let id = self.legacy_id();
                self.pending_simple.push((id, api_id, SimpleKind::Ok));
                vec![Outbound::Legacy(json!({"type": "cancel", "id": id}))]
            }
            "soft_interrupt" => {
                let id = self.legacy_id();
                self.pending_simple.push((id, api_id, SimpleKind::Ok));
                vec![Outbound::Legacy(json!({
                    "type": "soft_interrupt",
                    "id": id,
                    "content": request["content"].as_str().unwrap_or(""),
                    "urgent": request["urgent"].as_bool().unwrap_or(false),
                }))]
            }
            "clear" => {
                let id = self.legacy_id();
                self.pending_simple.push((id, api_id, SimpleKind::Ok));
                vec![Outbound::Legacy(json!({"type": "clear", "id": id}))]
            }
            "rewind" => {
                let id = self.legacy_id();
                self.pending_simple.push((id, api_id, SimpleKind::Ok));
                vec![Outbound::Legacy(json!({
                    "type": "rewind",
                    "id": id,
                    "message_index": request["message_index"].as_u64().unwrap_or(1),
                }))]
            }
            "get_history" => {
                let id = self.legacy_id();
                self.pending_simple.push((id, api_id, SimpleKind::History));
                vec![Outbound::Legacy(json!({"type": "get_history", "id": id}))]
            }
            // Answered from the stored record rather than the daemon: the
            // legacy protocol can only speak about the attached session, and
            // attaching to a session merely to read it would disturb the very
            // thing being previewed.
            "peek_session" => {
                let session_id = request["session_id"].as_str().unwrap_or("");
                let limit = request["limit"].as_u64().unwrap_or(PEEK_LIMIT) as usize;
                vec![Outbound::Reply(ServerFrame::reply(
                    api_id,
                    ApiEvent::History {
                        session_id: session_id.to_string(),
                        messages: Self::stored_tail(session_id, limit),
                        images: Vec::new(),
                    },
                ))]
            }
            // Answered locally before attach. The daemon treats `ping` as a
            // "lightweight control" request: when it arrives as the first
            // frame on a connection it is answered and the connection is then
            // closed, which would tear down the client's whole session. A
            // liveness probe must never cost the caller its connection, and
            // reaching the bridge already proves the socket is alive.
            "ping" if self.session_id.is_none() => {
                vec![Outbound::Reply(ServerFrame::reply(api_id, ApiEvent::Pong))]
            }
            "ping" => {
                let id = self.legacy_id();
                self.pending_simple.push((id, api_id, SimpleKind::Ping));
                vec![Outbound::Legacy(json!({"type": "ping", "id": id}))]
            }
            "list_sessions" => {
                let list_started = std::time::Instant::now();
                // A fresh API connection has not received a daemon `state`
                // snapshot. Start with every persisted record, then merge the
                // live snapshot so unattached dashboards and global event
                // subscribers discover complete session state.
                let limit = request["limit"].as_u64().map(|limit| limit as usize);
                let mut ids: BTreeSet<String> =
                    Self::stored_session_ids(limit).into_iter().collect();
                let ids_loaded = list_started.elapsed();
                ids.extend(self.known_sessions.iter().cloned());
                if let Some(attached) = self.session_id.clone() {
                    ids.insert(attached);
                }
                if let Some(limit) = limit {
                    let mut recent: Vec<_> = ids.into_iter().collect();
                    recent.sort_unstable_by_key(|id| {
                        std::cmp::Reverse(Self::session_modified_ms(id))
                    });
                    recent.truncate(limit);
                    ids = recent.into_iter().collect();
                }
                // Titles are deliberately not cached. A rename is persisted
                // before `SessionRenamed` is broadcast, and every list call
                // should reflect that newest canonical value even on another
                // API connection.
                let indexed_metadata: BTreeMap<_, _> = Self::recent_session_index_entries()
                    .into_iter()
                    .map(|entry| (entry.session_id.clone(), entry))
                    .collect();
                let metadata: BTreeMap<String, PersistedSessionMetadata> = ids
                    .iter()
                    .filter_map(|id| {
                        indexed_metadata
                            .get(id)
                            .map(PersistedSessionMetadata::from)
                            .or_else(|| Self::resolve_session_metadata(id))
                            .map(|metadata| (id.clone(), metadata))
                    })
                    .collect();
                let metadata_loaded = list_started.elapsed();
                for id in &ids {
                    if !self.session_dirs.contains_key(id)
                        && let Some(dir) = metadata
                            .get(id)
                            .and_then(|metadata| metadata.working_dir.clone())
                    {
                        self.session_dirs.insert(id.clone(), dir);
                    }
                }
                let _write_guard = Self::state_write_guard();
                let mut archive = Self::load_archive_state();
                if let Some(days) = archive.archive_after_days {
                    let cutoff = Self::now_ms().saturating_sub(u64::from(days) * 86_400_000);
                    let mut changed = false;
                    for id in &ids {
                        if self.session_id.as_ref() == Some(id) || archive.sessions.contains_key(id)
                        {
                            continue;
                        }
                        if Self::session_modified_ms(id).is_some_and(|modified| modified < cutoff) {
                            archive.sessions.insert(id.clone(), Self::now_ms());
                            changed = true;
                        }
                    }
                    if changed && let Err(message) = Self::save_archive_state(&archive) {
                        return Self::error_reply(api_id, ErrorCode::Internal, &message);
                    }
                }
                let include_archived = request["include_archived"].as_bool().unwrap_or(false);
                let sessions: Vec<_> = ids
                    .into_iter()
                    .filter(|session_id| {
                        include_archived || !archive.sessions.contains_key(session_id)
                    })
                    .map(|session_id| SessionInfo {
                        working_dir: self.session_dirs.get(&session_id).cloned(),
                        title: metadata
                            .get(&session_id)
                            .and_then(PersistedSessionMetadata::display_title),
                        status: if self.session_id.as_ref() == Some(&session_id) {
                            "attached".into()
                        } else {
                            "idle".into()
                        },
                        transcript_bytes: Self::transcript_bytes(&session_id),
                        saved: metadata.get(&session_id).is_some_and(|value| value.saved),
                        updated_at_ms: metadata
                            .get(&session_id)
                            .and_then(|value| value.updated_at_ms)
                            .or_else(|| {
                                Self::session_modified_ms(&session_id).map(|value| value as i64)
                            }),
                        last_active_at_ms: metadata
                            .get(&session_id)
                            .and_then(|value| value.last_active_at_ms),
                        archived: archive.sessions.contains_key(&session_id),
                        archived_at_ms: archive.sessions.get(&session_id).copied(),
                        session_id,
                    })
                    .collect();
                let completed = list_started.elapsed();
                eprintln!(
                    "harness API bridge: list_sessions ids={:.1}ms metadata={:.1}ms total={:.1}ms count={}",
                    ids_loaded.as_secs_f64() * 1_000.0,
                    metadata_loaded.as_secs_f64() * 1_000.0,
                    completed.as_secs_f64() * 1_000.0,
                    sessions.len()
                );
                vec![Outbound::Reply(ServerFrame::reply(
                    api_id,
                    ApiEvent::Sessions { sessions },
                ))]
            }
            // Answered from the cached catalog. The daemon pushes it on attach
            // and on every change, so asking again would add a round trip to
            // an interaction (opening a picker) that must feel instant.
            "list_models" => {
                // Attach pushes the catalog, but a client that asks in the
                // same breath as attaching can beat it. Returning an empty
                // list would look like "no models exist", so ask the daemon
                // and answer when the catalog lands.
                if self.available_models.is_empty() {
                    let id = self.legacy_id();
                    self.pending_simple.push((id, api_id, SimpleKind::Models));
                    return vec![Outbound::Legacy(
                        json!({"type": "get_model_catalog", "id": id}),
                    )];
                }
                vec![Outbound::Reply(ServerFrame::reply(
                    api_id,
                    ApiEvent::Models {
                        session_id: self.session_id.as_deref().unwrap_or("").to_owned(),
                        models: self.available_models.clone(),
                        current: self.current_model.clone(),
                    },
                ))]
            }
            "get_runtime_info" => vec![Outbound::Reply(ServerFrame::reply(
                api_id,
                ApiEvent::RuntimeInfo {
                    session_id: self.session_id.as_deref().unwrap_or("").to_owned(),
                    provider: self.current_provider.clone(),
                    model: self.current_model.clone(),
                    reasoning_effort: self.current_effort.clone(),
                    routes: self.available_routes.clone(),
                },
            ))],
            "set_api_key" | "clear_api_key" => {
                let provider = request["provider"].as_str().unwrap_or("");
                let Some((provider, env_keys, file_name)) = Self::credential_binding(provider)
                else {
                    return Self::error_reply(
                        api_id,
                        ErrorCode::InvalidRequest,
                        "unsupported API-key provider; supported: claude-api, openai-api, openrouter, cursor, gemini, jcode",
                    );
                };
                let configured = req == "set_api_key";
                let key = request["api_key"].as_str().unwrap_or("");
                if configured
                    && (key.trim().is_empty()
                        || key.trim() != key
                        || key.contains(['\n', '\r', '\0']))
                {
                    return Self::error_reply(
                        api_id,
                        ErrorCode::InvalidRequest,
                        "api_key must be a non-empty, trimmed, non-NUL single line",
                    );
                }
                if let Err(message) =
                    Self::write_credential(file_name, env_keys, configured.then_some(key))
                {
                    return Self::error_reply(api_id, ErrorCode::Internal, &message);
                }
                let id = self.legacy_id();
                self.pending_simple.push((
                    id,
                    api_id,
                    SimpleKind::Credential {
                        provider: provider.to_string(),
                        configured,
                    },
                ));
                vec![Outbound::Legacy(json!({
                    "type": "notify_auth_changed",
                    "id": id,
                    "provider": provider,
                    "auth": {
                        "provider": provider,
                        "credential_source": "api_key_file",
                        "auth_method": "remote_tui_paste_api_key"
                    }
                }))]
            }
            "read_file" => {
                let session_id = request["session_id"].as_str().unwrap_or("");
                let relative = request["path"].as_str().unwrap_or("");
                let max = request["max_bytes"]
                    .as_u64()
                    .unwrap_or(DEFAULT_FILE_BYTES)
                    .min(MAX_FILE_BYTES);
                match Self::read_session_file(session_id, relative, max) {
                    Ok((content, size, truncated)) => vec![Outbound::Reply(ServerFrame::reply(
                        api_id,
                        ApiEvent::FileContent {
                            session_id: session_id.to_string(),
                            path: relative.to_string(),
                            content,
                            size,
                            truncated,
                        },
                    ))],
                    Err((code, message)) => Self::error_reply(api_id, code, &message),
                }
            }
            "find_files" => {
                let session_id = request["session_id"].as_str().unwrap_or("");
                let query = request["query"].as_str().unwrap_or("");
                let limit = request["limit"]
                    .as_u64()
                    .unwrap_or(DEFAULT_FIND_LIMIT as u64)
                    .min(MAX_FIND_LIMIT as u64) as usize;
                match Self::find_session_files(session_id, query, limit) {
                    Ok(paths) => vec![Outbound::Reply(ServerFrame::reply(
                        api_id,
                        ApiEvent::Files {
                            session_id: session_id.to_string(),
                            paths,
                        },
                    ))],
                    Err((code, message)) => Self::error_reply(api_id, code, &message),
                }
            }
            "search_text" => {
                let session_id = request["session_id"].as_str().unwrap_or("");
                let query = request["query"].as_str().unwrap_or("");
                let under = request["path"].as_str();
                let limit = request["limit"]
                    .as_u64()
                    .unwrap_or(DEFAULT_FIND_LIMIT as u64)
                    .min(MAX_FIND_LIMIT as u64) as usize;
                match Self::search_session_text(session_id, query, under, limit) {
                    Ok(matches) => vec![Outbound::Reply(ServerFrame::reply(
                        api_id,
                        ApiEvent::TextMatches {
                            session_id: session_id.to_string(),
                            matches,
                        },
                    ))],
                    Err((code, message)) => Self::error_reply(api_id, code, &message),
                }
            }
            "file_status" => {
                let session_id = request["session_id"].as_str().unwrap_or("");
                let relative = request["path"].as_str().unwrap_or("");
                match Self::session_file_status(session_id, relative) {
                    Ok((exists, kind, size, modified_ms)) => {
                        vec![Outbound::Reply(ServerFrame::reply(
                            api_id,
                            ApiEvent::FileStatus {
                                session_id: session_id.to_string(),
                                path: relative.to_string(),
                                exists,
                                kind,
                                size,
                                modified_ms,
                            },
                        ))]
                    }
                    Err((code, message)) => Self::error_reply(api_id, code, &message),
                }
            }
            "set_model" => {
                let model = request["model"].as_str().unwrap_or("");
                if model.is_empty() {
                    return vec![Outbound::Reply(ServerFrame::reply(
                        api_id,
                        ApiEvent::Error {
                            code: ErrorCode::InvalidRequest,
                            message: "set_model needs a non-empty `model`".into(),
                        },
                    ))];
                }
                let id = self.legacy_id();
                self.pending_simple.push((id, api_id, SimpleKind::Model));
                vec![Outbound::Legacy(json!({
                    "type": "set_model",
                    "id": id,
                    "model": model,
                }))]
            }
            "set_reasoning_effort" => {
                let effort = request["effort"].as_str().unwrap_or("");
                if effort.is_empty() {
                    return vec![Outbound::Reply(ServerFrame::reply(
                        api_id,
                        ApiEvent::Error {
                            code: ErrorCode::InvalidRequest,
                            message: "set_reasoning_effort needs a non-empty `effort`".into(),
                        },
                    ))];
                }
                let id = self.legacy_id();
                self.pending_simple
                    .push((id, api_id, SimpleKind::ReasoningEffort));
                vec![Outbound::Legacy(json!({
                    "type": "set_reasoning_effort",
                    "id": id,
                    "effort": effort,
                }))]
            }
            "compact" => {
                let id = self.legacy_id();
                self.pending_simple.push((id, api_id, SimpleKind::Compact));
                vec![Outbound::Legacy(json!({"type": "compact", "id": id}))]
            }
            "rename_session" => {
                let id = self.legacy_id();
                self.pending_simple.push((id, api_id, SimpleKind::Ok));
                let mut rename = json!({"type": "rename_session", "id": id});
                // An absent title clears it and restores the generated one, so
                // null and "" must stay distinguishable on the wire.
                if let Some(title) = request["title"].as_str() {
                    rename["title"] = json!(title);
                }
                vec![Outbound::Legacy(rename)]
            }
            "rewind_undo" => {
                let id = self.legacy_id();
                self.pending_simple.push((id, api_id, SimpleKind::Ok));
                vec![Outbound::Legacy(json!({"type": "rewind_undo", "id": id}))]
            }
            "cancel_soft_interrupts" => {
                let id = self.legacy_id();
                self.pending_simple.push((id, api_id, SimpleKind::Ok));
                vec![Outbound::Legacy(
                    json!({"type": "cancel_soft_interrupts", "id": id}),
                )]
            }
            "detach_session" => {
                let id = self.legacy_id();
                vec![
                    Outbound::Legacy(json!({"type": "prepare_disconnect", "id": id})),
                    Outbound::Reply(ServerFrame::reply(api_id, ApiEvent::Ok)),
                ]
            }
            "permission_response" => {
                // The legacy protocol does not surface permission prompts on
                // this path, so the bridge never emits `permission_request`
                // and there is nothing for a response to answer. Say that,
                // rather than "not supported", which reads like a bug the
                // caller should work around. Clients discover this up front
                // via the absence of the `permissions` capability in `hello`.
                vec![Outbound::Reply(ServerFrame::reply(
                    api_id,
                    ApiEvent::Error {
                        code: ErrorCode::InvalidRequest,
                        message: "this server does not issue permission prompts \
                                  (no `permissions` capability), so there is nothing to respond to"
                            .into(),
                    },
                ))]
            }
            other => vec![Outbound::Reply(ServerFrame::reply(
                api_id,
                ApiEvent::Error {
                    code: ErrorCode::UnknownRequest,
                    message: format!("unknown request: {other}"),
                },
            ))],
        }
    }
}
