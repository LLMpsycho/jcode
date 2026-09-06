use super::*;

impl JcodeClient {
    /// Models this session can switch to, and which one is serving it.
    pub fn list_models(&self, session_id: &str) -> Result<(Vec<String>, Option<String>)> {
        match self
            .request_ok(ApiRequest::ListModels {
                session_id: session_id.to_string(),
            })?
            .event
        {
            ApiEvent::Models {
                models, current, ..
            } => Ok((models, current)),
            other => Err(unexpected("models", &other)),
        }
    }

    /// Runtime identity, route catalog, protocol metadata, and a live health
    /// check for a session.
    pub fn get_runtime_info(&self, session_id: &str) -> Result<RuntimeInfo> {
        self.ping()?;
        match self
            .request_ok(ApiRequest::GetRuntimeInfo {
                session_id: session_id.to_string(),
            })?
            .event
        {
            ApiEvent::RuntimeInfo {
                session_id,
                provider,
                model,
                reasoning_effort,
                routes,
            } => {
                let mut providers = Vec::new();
                if let Some(provider) = provider.as_ref() {
                    providers.push(provider.clone());
                }
                for route in &routes {
                    if !providers.contains(&route.provider) {
                        providers.push(route.provider.clone());
                    }
                }
                Ok(RuntimeInfo {
                    server: self.server.clone(),
                    protocol_version: API_VERSION_MAJOR,
                    capabilities: self.capabilities.clone(),
                    healthy: true,
                    session_id,
                    provider,
                    model,
                    reasoning_effort,
                    providers,
                    routes,
                })
            }
            other => Err(unexpected("runtime_info", &other)),
        }
    }

    /// Persist an API key in jcode's owner-only provider store and hot-reload
    /// provider credentials.
    pub fn set_api_key(&self, provider: &str, api_key: &str) -> Result<()> {
        match self
            .request_ok(ApiRequest::SetApiKey {
                provider: provider.to_string(),
                api_key: api_key.to_string(),
            })?
            .event
        {
            ApiEvent::CredentialUpdated { .. } => Ok(()),
            other => Err(unexpected("credential_updated", &other)),
        }
    }

    /// Remove a persisted API-key credential and hot-reload provider
    /// credentials.
    pub fn clear_api_key(&self, provider: &str) -> Result<()> {
        match self
            .request_ok(ApiRequest::ClearApiKey {
                provider: provider.to_string(),
            })?
            .event
        {
            ApiEvent::CredentialUpdated { .. } => Ok(()),
            other => Err(unexpected("credential_updated", &other)),
        }
    }

    /// Read one UTF-8 file under the session working directory.
    pub fn read_file(
        &self,
        session_id: &str,
        path: &str,
        max_bytes: Option<u64>,
    ) -> Result<FileContent> {
        match self
            .request_ok(ApiRequest::ReadFile {
                session_id: session_id.to_string(),
                path: path.to_string(),
                max_bytes,
            })?
            .event
        {
            ApiEvent::FileContent {
                path,
                content,
                size,
                truncated,
                ..
            } => Ok(FileContent {
                path,
                content,
                size,
                truncated,
            }),
            other => Err(unexpected("file_content", &other)),
        }
    }

    /// Find files by case-insensitive path substring under the session root.
    pub fn find_files(
        &self,
        session_id: &str,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<String>> {
        match self
            .request_ok(ApiRequest::FindFiles {
                session_id: session_id.to_string(),
                query: query.to_string(),
                limit,
            })?
            .event
        {
            ApiEvent::Files { paths, .. } => Ok(paths),
            other => Err(unexpected("files", &other)),
        }
    }

    /// Search UTF-8 files under the session root for a literal string.
    pub fn search_text(
        &self,
        session_id: &str,
        query: &str,
        options: SearchTextOptions,
    ) -> Result<Vec<TextMatch>> {
        match self
            .request_ok(ApiRequest::SearchText {
                session_id: session_id.to_string(),
                query: query.to_string(),
                path: options.path,
                limit: options.limit,
            })?
            .event
        {
            ApiEvent::TextMatches { matches, .. } => Ok(matches),
            other => Err(unexpected("text_matches", &other)),
        }
    }

    /// Read safe filesystem metadata for a path under the session root.
    pub fn file_status(&self, session_id: &str, path: &str) -> Result<FileStatus> {
        match self
            .request_ok(ApiRequest::FileStatus {
                session_id: session_id.to_string(),
                path: path.to_string(),
            })?
            .event
        {
            ApiEvent::FileStatus {
                path,
                exists,
                kind,
                size,
                modified_ms,
                ..
            } => Ok(FileStatus {
                path,
                exists,
                kind,
                size,
                modified_ms,
            }),
            other => Err(unexpected("file_status", &other)),
        }
    }

    /// Switch the session to a different model. `model` is an id from
    /// `list_models`.
    pub fn set_model(&self, session_id: &str, model: &str) -> Result<()> {
        self.request_ok(ApiRequest::SetModel {
            session_id: session_id.to_string(),
            model: model.to_string(),
        })
        .map(drop)
    }

    /// Set how much the model deliberates before answering. The accepted set
    /// is per-provider, so this takes a string rather than a union that would
    /// go stale.
    pub fn set_reasoning_effort(&self, session_id: &str, effort: &str) -> Result<()> {
        self.request_ok(ApiRequest::SetReasoningEffort {
            session_id: session_id.to_string(),
            effort: effort.to_string(),
        })
        .map(drop)
    }

    /// Control the advisor using the session's existing provider sign-ins.
    /// Inspect the returned `error` before reporting a durable change.
    pub fn advisor(
        &self,
        session_id: &str,
        request: jcode_harness_api::AdvisorRequest,
    ) -> Result<jcode_harness_api::AdvisorControlResult> {
        match self
            .request_ok(ApiRequest::Advisor {
                session_id: session_id.to_string(),
                request,
            })?
            .event
        {
            ApiEvent::AdvisorResult { result, .. } => Ok(result),
            other => Err(unexpected("advisor_result", &other)),
        }
    }
}
