use super::*;

pub async fn run_model_command(
    choice: &crate::cli::provider_init::ProviderChoice,
    model: Option<&str>,
    emit_json: bool,
    verbose: bool,
) -> Result<()> {
    let provider = crate::cli::provider_init::init_provider_quiet(choice, model).await?;

    if let Err(err) = provider.prefetch_models().await
        && !crate::cli::output::quiet_enabled()
    {
        eprintln!("Warning: failed to refresh dynamic model list: {}", err);
    }

    let routes = provider.model_routes();
    let filtered_routes = filter_cli_model_routes_for_choice(choice, &routes);
    let models = if filtered_routes.len() == routes.len() {
        collect_cli_model_names(&routes, provider.available_models_display())
    } else {
        collect_cli_model_names(&filtered_routes, Vec::new())
    };

    if models.is_empty() {
        anyhow::bail!(
            "No models found for provider '{}'. Check credentials or try a different --provider.",
            provider.name()
        );
    }

    if emit_json {
        let provider_label = crate::cli::provider_init::login_provider_for_choice(choice)
            .map(|provider| provider.display_name.to_string())
            .unwrap_or_else(|| {
                crate::provider_catalog::runtime_provider_display_name(provider.name())
            });
        let report = ModelListReport {
            provider: provider_label,
            selected_model: provider.model(),
            models,
            routes: filtered_routes
                .iter()
                .map(|route| ModelListRouteReport {
                    provider: cli_route_provider_display(&route.provider, &route.api_method),
                    model: route.model.clone(),
                    method: cli_api_method_display(&route.api_method),
                    available: route.available,
                })
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        if verbose {
            println!(
                "Provider: {}",
                crate::provider_catalog::runtime_provider_display_name(provider.name())
            );
            println!("Selected model: {}", provider.model());
            println!("Available models: {}", models.len());
            println!();
        }
        for model in models {
            println!("{}", model);
        }
    }

    Ok(())
}

pub(super) fn cli_api_method_display(raw: &str) -> String {
    crate::provider::ModelRouteApiMethod::parse(raw).display_label()
}

pub(super) fn cli_route_provider_display(provider: &str, api_method: &str) -> String {
    if crate::provider::ModelRouteApiMethod::parse(api_method).is_openrouter()
        && provider != "auto"
        && !provider.contains("OpenRouter")
    {
        format!("OpenRouter/{}", provider)
    } else {
        provider.to_string()
    }
}

pub(super) fn collect_cli_model_names(
    routes: &[crate::provider::ModelRoute],
    display_models: Vec<String>,
) -> Vec<String> {
    let mut deduped = Vec::new();
    let mut seen = BTreeSet::new();

    fn push_model(deduped: &mut Vec<String>, seen: &mut BTreeSet<String>, model: &str) {
        let trimmed = model.trim();
        if !crate::provider::is_listable_model_name(trimmed) {
            return;
        }
        if seen.insert(trimmed.to_string()) {
            deduped.push(trimmed.to_string());
        }
    }

    for route in routes.iter().filter(|route| route.available) {
        push_model(&mut deduped, &mut seen, &route.model);
    }

    if deduped.is_empty() {
        for route in routes {
            push_model(&mut deduped, &mut seen, &route.model);
        }
    }

    for model in display_models {
        push_model(&mut deduped, &mut seen, &model);
    }

    deduped
}

#[allow(deprecated)]
pub(super) fn filter_cli_model_routes_for_choice(
    choice: &crate::cli::provider_init::ProviderChoice,
    routes: &[crate::provider::ModelRoute],
) -> Vec<crate::provider::ModelRoute> {
    use crate::cli::provider_init::ProviderChoice;

    let keep = |route: &&crate::provider::ModelRoute| match choice {
        ProviderChoice::Claude | ProviderChoice::ClaudeSubprocess => {
            route.api_method_kind().is_anthropic_credential_route()
        }
        ProviderChoice::Openai => {
            let method = route.api_method_kind();
            matches!(method, crate::provider::ModelRouteApiMethod::OpenAIOAuth)
                || matches!(method, crate::provider::ModelRouteApiMethod::Other(ref value) if value == "chatgpt-web")
        }
        ProviderChoice::OpenaiApi => matches!(
            route.api_method_kind(),
            crate::provider::ModelRouteApiMethod::OpenAIApiKey
        ),
        ProviderChoice::Openrouter | ProviderChoice::Azure => {
            route.api_method_kind().is_openrouter()
        }
        ProviderChoice::Copilot => route.api_method_kind().is_copilot(),
        _ => true,
    };

    let filtered: Vec<_> = routes.iter().filter(keep).cloned().collect();
    if filtered.is_empty() {
        routes.to_vec()
    } else {
        filtered
    }
}
