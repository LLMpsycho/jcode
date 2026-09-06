use super::*;

impl OpenAIProvider {
    /// Shared request settings keep speculative warmup and foreground generation
    /// identical even when reasoning, tools, cache policy, or service tier change.
    pub(super) async fn response_request(
        &self,
        input: &[Value],
        tools: &[ToolDefinition],
        system: &str,
    ) -> Value {
        let model_id = self.model_id().await;
        let is_chatgpt_mode = Self::is_chatgpt_mode(&*self.credentials.read().await);
        self.response_request_for_model(&model_id, input, tools, system, is_chatgpt_mode)
    }

    pub(super) fn response_request_for_model(
        &self,
        model_id: &str,
        input: &[Value],
        tools: &[ToolDefinition],
        system: &str,
        is_chatgpt_mode: bool,
    ) -> Value {
        let api_tools = build_tools(tools);
        let reasoning_effort = self
            .reasoning_effort
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
            // No explicit user effort: fall back to the model's jcode-side
            // default (e.g. `low` for GPT-5.6 Sol).
            .or_else(|| Self::default_reasoning_effort_for_model(model_id));
        // Map the `swarm` sentinel (and any future aliases) to the real effort
        // value the API understands.
        let api_reasoning_effort = self.api_reasoning_effort(reasoning_effort.as_deref());
        let service_tier = self
            .service_tier
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
        let native_compaction_threshold =
            self.native_compaction_threshold_for_context_window(self.context_window());
        let mut request = Self::build_response_request(
            model_id,
            system.to_string(),
            input,
            &api_tools,
            is_chatgpt_mode,
            self.max_output_tokens,
            api_reasoning_effort.as_deref(),
            service_tier.as_deref(),
            self.prompt_cache_key.as_deref(),
            self.prompt_cache_retention.as_deref(),
            native_compaction_threshold,
        );
        self.apply_explicit_tool_policy(&mut request);
        request
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "request construction threads explicit per-request OpenAI settings without hidden state"
    )]
    pub(super) fn build_response_request(
        model_id: &str,
        instructions: String,
        input: &[Value],
        api_tools: &[Value],
        is_chatgpt_mode: bool,
        max_output_tokens: Option<u32>,
        reasoning_effort: Option<&str>,
        service_tier: Option<&str>,
        prompt_cache_key: Option<&str>,
        prompt_cache_retention: Option<&str>,
        native_compaction_threshold: Option<usize>,
    ) -> Value {
        let mut tools = api_tools.to_vec();
        // The hosted `image_generation` tool is only available to general
        // ChatGPT/GPT models on the Responses backend. Codex models
        // (`*-codex*`) reject unknown hosted tools, so don't attach it for them.
        // Empty tool lists also disable hosted tools. Investigative helpers
        // additionally restrict nonempty lists before transport dispatch.
        if !api_tools.is_empty() && is_chatgpt_mode && model_supports_image_generation(model_id) {
            tools.push(serde_json::json!({ "type": "image_generation" }));
        }

        let mut request = serde_json::json!({
            "model": model_id,
            "instructions": instructions,
            "input": input,
            "tools": tools,
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "stream": true,
            "store": false,
            "include": ["reasoning.encrypted_content"],
        });

        if !is_chatgpt_mode && let Some(max_output_tokens) = max_output_tokens {
            request["max_output_tokens"] = serde_json::json!(max_output_tokens);
        }

        if let Some(effort) = reasoning_effort {
            request["reasoning"] = openai_stream_runtime::reasoning_payload(effort);
        }

        if let Some(service_tier) = service_tier {
            request["service_tier"] = serde_json::json!(service_tier);
        }

        if let Some(compact_threshold) = native_compaction_threshold {
            request["context_management"] = serde_json::json!([
                {
                    "type": "compaction",
                    "compact_threshold": compact_threshold,
                }
            ]);
        }

        if !is_chatgpt_mode {
            if let Some(key) = prompt_cache_key {
                request["prompt_cache_key"] = serde_json::json!(key);
            }
            if let Some(retention) =
                Self::effective_prompt_cache_retention(model_id, prompt_cache_retention)
            {
                request["prompt_cache_retention"] = serde_json::json!(retention);
            }
        }

        request
    }

    pub(super) fn apply_explicit_tool_policy(&self, request: &mut Value) {
        if self.explicit_tools_only.load(AtomicOrdering::Relaxed)
            && let Some(tools) = request.get_mut("tools").and_then(Value::as_array_mut)
        {
            tools.retain(|tool| tool.get("type").and_then(Value::as_str) == Some("function"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advisor_explicit_tools_exclude_hosted_tools_without_removing_investigation() {
        let _lock = jcode_base::storage::lock_test_env();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _entered = runtime.enter();
        let provider = OpenAIProvider::new(CodexCredentials {
            access_token: "test-explicit-tool-policy".into(),
            refresh_token: String::new(),
            id_token: None,
            account_id: None,
            expires_at: None,
        });
        let functions = serde_json::json!([
            {"type":"function", "name":"read"},
            {"type":"function", "name":"advise"}
        ]);
        let mut ordinary = OpenAIProvider::build_response_request(
            "gpt-5.5",
            "Review".into(),
            &[],
            functions.as_array().unwrap(),
            true,
            None,
            Some("high"),
            None,
            None,
            None,
            None,
        );
        provider.apply_explicit_tool_policy(&mut ordinary);
        assert!(
            ordinary["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["type"] == "image_generation")
        );
        provider.restrict_to_explicit_tools().unwrap();
        let tools = [ToolDefinition {
            name: "read".into(),
            description: "Read evidence".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let warmed = provider.response_request_for_model("gpt-5.5", &[], &tools, "advisor", true);
        let advertised = warmed["tools"].as_array().unwrap();
        assert_eq!(advertised.len(), 1);
        assert_eq!(advertised[0]["name"], "read");
        assert_eq!(advertised[0]["type"], "function");
        let mut advisor = ordinary.clone();
        // Future hosted tool kinds must be excluded too, not just image generation.
        advisor["tools"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"type":"web_search"}));
        provider.apply_explicit_tool_policy(&mut advisor);
        assert_eq!(advisor["tools"], functions);
        assert_eq!(advisor["reasoning"]["effort"], "high");
        assert_eq!(ordinary["tools"].as_array().unwrap().len(), 3);
    }
}
