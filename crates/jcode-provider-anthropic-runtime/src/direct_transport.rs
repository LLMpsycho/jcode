use super::*;

fn configured_env(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("{name} must contain valid Unicode")
        }
    }
}

pub(super) fn direct_api_url() -> Result<String> {
    let base = match configured_env("JCODE_ANTHROPIC_API_BASE")? {
        Some(base) => Some(base),
        None => configured_env("ANTHROPIC_BASE_URL")?,
    }
    .map(|value| value.trim().trim_end_matches('/').to_string())
    .filter(|value| !value.is_empty());
    Ok(match base {
        Some(base) if base.ends_with("/messages") => base,
        Some(base) => format!("{base}/messages"),
        None => API_URL.to_string(),
    })
}

pub(super) fn configured_direct_headers() -> Result<HeaderMap> {
    let Some(raw) =
        configured_env("JCODE_ANTHROPIC_HEADERS")?.filter(|value| !value.trim().is_empty())
    else {
        return Ok(HeaderMap::new());
    };
    let headers: std::collections::BTreeMap<String, String> = serde_json::from_str(&raw)
        .context("JCODE_ANTHROPIC_HEADERS must be a JSON object of string values")?;
    let mut result = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .with_context(|| format!("invalid Anthropic-compatible header name '{name}'"))?;
        let value = HeaderValue::from_str(&value)
            .with_context(|| format!("invalid value for Anthropic-compatible header '{name}'"))?;
        result.insert(name, value);
    }
    Ok(result)
}

pub(super) fn direct_auth_mode() -> Result<String> {
    let explicit = configured_env("JCODE_ANTHROPIC_AUTH")?.filter(|value| !value.trim().is_empty());
    let mode = match explicit {
        Some(mode) => mode,
        None if configured_env("ANTHROPIC_AUTH_TOKEN")?
            .is_some_and(|value| !value.trim().is_empty()) =>
        {
            "bearer".to_string()
        }
        None => "header".to_string(),
    };
    Ok(mode.trim().to_ascii_lowercase())
}

#[derive(Clone)]
pub(super) struct DirectTransportConfig {
    pub(super) api_url: String,
    pub(super) headers: std::result::Result<HeaderMap, String>,
    pub(super) auth_mode: String,
    pub(super) auth_header: String,
    #[cfg(test)]
    pub(super) oauth_fixture_url: Option<String>,
}

impl DirectTransportConfig {
    pub(super) fn from_env() -> Self {
        Self::try_from_env().unwrap_or_else(|err| Self {
            api_url: API_URL.to_string(),
            // Provider construction is infallible. Retain the configuration
            // failure so direct requests stop before credentials are sent.
            headers: Err(format!("{err:#}")),
            auth_mode: "header".to_string(),
            auth_header: "x-api-key".to_string(),
            #[cfg(test)]
            oauth_fixture_url: None,
        })
    }

    fn try_from_env() -> Result<Self> {
        Ok(Self {
            api_url: direct_api_url()?,
            headers: Ok(configured_direct_headers()?),
            auth_mode: direct_auth_mode()?,
            auth_header: configured_env("JCODE_ANTHROPIC_AUTH_HEADER")?
                .unwrap_or_else(|| "x-api-key".to_string()),
            #[cfg(test)]
            oauth_fixture_url: None,
        })
    }
}
