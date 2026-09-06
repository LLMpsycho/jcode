use crate::protocol::{AuthChanged, CatalogNamespace, RuntimeProviderKey};
use crate::provider::ModelRoute;
use crate::provider::activation::{ProviderActivation, RuntimeProviderId};
use jcode_provider_core::ActiveProvider;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthActivationRequest {
    pub legacy_provider_hint: Option<String>,
    pub auth: Option<AuthChanged>,
}

impl AuthActivationRequest {
    pub fn new(legacy_provider_hint: Option<String>, auth: Option<AuthChanged>) -> Self {
        Self {
            legacy_provider_hint,
            auth,
        }
    }

    pub fn provider_id(&self) -> Option<String> {
        self.auth
            .as_ref()
            .map(|auth| auth.provider.as_str().to_string())
            .or_else(|| self.legacy_provider_hint.clone())
            .and_then(|provider| {
                normalized_auth_provider_id(Some(provider.as_str())).map(str::to_string)
            })
    }

    pub fn expected_runtime(&self) -> Option<&RuntimeProviderKey> {
        self.auth
            .as_ref()
            .and_then(|auth| auth.expected_runtime.as_ref())
    }

    pub fn expected_catalog_namespace(&self) -> Option<&CatalogNamespace> {
        self.auth
            .as_ref()
            .and_then(|auth| auth.expected_catalog_namespace.as_ref())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthActivationResult {
    pub provider_id: Option<String>,
    pub provider_label: Option<String>,
    pub activated_model: Option<String>,
    pub expected_runtime: Option<String>,
    pub expected_catalog_namespace: Option<String>,
}

impl AuthActivationResult {
    pub fn model_switch_request(&self, current_provider_name: &str, model: &str) -> String {
        model_switch_request_for_provider_id(
            self.provider_id.as_deref(),
            current_provider_name,
            model,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthCatalogInvariantReport {
    pub applicable: bool,
    pub provider_id: Option<String>,
    pub provider_label: Option<String>,
    pub selectable_provider_routes: usize,
    pub selected_model: Option<String>,
    pub selected_model_matches_provider_route: bool,
    pub route_sample: Vec<String>,
}

impl AuthCatalogInvariantReport {
    pub fn ok(&self) -> bool {
        !self.applicable
            || (self.selectable_provider_routes > 0 && self.selected_model_matches_provider_route)
    }

    pub fn warning_message(&self) -> Option<String> {
        if self.ok() {
            return None;
        }

        let provider = self
            .provider_label
            .as_deref()
            .or(self.provider_id.as_deref())
            .unwrap_or("provider");
        let selected = self.selected_model.as_deref().unwrap_or("none");
        let sample = if self.route_sample.is_empty() {
            "none".to_string()
        } else {
            self.route_sample.join(", ")
        };
        Some(format!(
            "\n\n**Auth Model Catalog Warning**\n\nExpected selectable {provider} model routes after auth, but found {} matching route(s). Selected model: `{selected}`. Matching route sample: {sample}.",
            self.selectable_provider_routes
        ))
    }
}

pub fn validate_catalog_invariants(
    activation: &AuthActivationResult,
    selected_model: Option<&str>,
    routes: &[ModelRoute],
) -> AuthCatalogInvariantReport {
    let provider_id = activation.provider_id.clone();
    let provider_label = activation.provider_label.clone();
    let applicable = provider_id.is_some() || provider_label.is_some();
    let selected_model = selected_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToString::to_string);

    let matching_routes = routes
        .iter()
        .filter(|route| route.available && route_matches_activation(route, activation))
        .collect::<Vec<_>>();
    let selected_model_matches_provider_route = selected_model
        .as_ref()
        .is_some_and(|selected| matching_routes.iter().any(|route| route.model == *selected));
    let route_sample = matching_routes
        .iter()
        .take(5)
        .map(|route| format!("`{}` via {}", route.model, route.api_method))
        .collect::<Vec<_>>();

    AuthCatalogInvariantReport {
        applicable,
        provider_id,
        provider_label,
        selectable_provider_routes: matching_routes.len(),
        selected_model,
        selected_model_matches_provider_route,
        route_sample,
    }
}

pub fn provider_model_to_select_after_auth(
    activation: &AuthActivationResult,
    selected_model: Option<&str>,
    routes: &[ModelRoute],
) -> Option<String> {
    let matching_routes = routes
        .iter()
        .filter(|route| route.available && route_matches_activation(route, activation))
        .collect::<Vec<_>>();
    if matching_routes.is_empty() {
        return None;
    }

    let selected_model = selected_model
        .map(str::trim)
        .filter(|model| !model.is_empty());
    if let Some(selected) = selected_model
        && matching_routes.iter().any(|route| route.model == selected)
    {
        let same_model_wrong_route_exists = routes.iter().any(|route| {
            route.available
                && route.model == selected
                && !route_matches_activation(route, activation)
        });
        if same_model_wrong_route_exists {
            return Some(selected.to_string());
        }
        return None;
    }

    if let Some(activated_model) = activation
        .activated_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        && matching_routes
            .iter()
            .any(|route| route.model == activated_model)
    {
        return Some(activated_model.to_string());
    }

    // No usable current model and no activation-supplied model: fall back to the
    // best available route. Plain catalog order would pick whatever the live
    // catalog happened to list first (e.g. `claude-haiku-4-5-...` ahead of
    // `claude-opus-4-8`), so an Anthropic API-key login would auto-select Haiku
    // instead of the provider's flagship default. When the provider has a
    // curated flagship-first preference order, pick the highest-ranked matching
    // route; ties and unranked providers preserve catalog order.
    let orders = provider_preferred_model_orders(activation);
    if !orders.is_empty() {
        // 1. Auto-promote a brand-new frontier release that is not yet in the
        //    curated list. Model ids in these families encode their version
        //    (`gpt-5.5`, `claude-opus-4-8`), so a strictly-newer *pure* flagship
        //    id (no cheap/specialized suffix) should become the default the day
        //    it ships, without waiting for a code change.
        if let Some(newer) = newest_frontier_release(activation, &matching_routes) {
            return Some(newer);
        }
        // 2. Otherwise pick the best curated model by quality order.
        return matching_routes
            .iter()
            .min_by_key(|route| preferred_model_rank(orders, &route.model))
            .map(|route| route.model.clone());
    }

    if let Some(provider_id) = activation.provider_id.as_deref()
        && let Some(newest_model) =
            crate::provider_catalog::newest_released_model_for_openai_compatible_profile(
                provider_id,
            )
        && matching_routes
            .iter()
            .any(|route| route.model == newest_model)
    {
        return Some(newest_model);
    }

    matching_routes.first().map(|route| route.model.clone())
}

/// Reconcile a provider after auth while preserving an explicit configured
/// default when that model is available for the activated provider.
pub fn provider_model_to_select_after_auth_with_configured_default(
    activation: &AuthActivationResult,
    configured_model: Option<&str>,
    selected_model: Option<&str>,
    routes: &[ModelRoute],
) -> Option<String> {
    let configured_model = configured_model
        .map(str::trim)
        .filter(|model| !model.is_empty());
    if let Some(configured) = configured_model
        && routes.iter().any(|route| {
            route.available
                && route.model == configured
                && route_matches_activation(route, activation)
        })
    {
        if selected_model.map(str::trim) != Some(configured) {
            return Some(configured.to_string());
        }
    }

    provider_model_to_select_after_auth(activation, selected_model, routes)
}

/// Pick the strongest available route across every authenticated provider.
///
/// This is intentionally separate from [`provider_model_to_select_after_auth`],
/// which keeps normal re-authentication scoped to the provider that changed.
/// First-run onboarding can use this global selector after importing multiple
/// accounts. Returning the complete route preserves OAuth/API-key/profile
/// identity when the caller applies the selection.
pub fn globally_preferred_default_route(routes: &[ModelRoute]) -> Option<ModelRoute> {
    routes
        .iter()
        .enumerate()
        .filter(|(_, route)| route.available)
        .min_by_key(|(catalog_index, route)| {
            (globally_preferred_model_rank(&route.model), *catalog_index)
        })
        .map(|(_, route)| route.clone())
}

fn globally_preferred_model_rank(model: &str) -> (u8, usize) {
    let normalized = normalize_model_for_preference(model);
    let openai_default = normalize_model_for_preference(jcode_provider_core::DEFAULT_OPENAI_MODEL);
    let claude_default = normalize_model_for_preference(jcode_provider_core::DEFAULT_CLAUDE_MODEL);

    if normalized == openai_default {
        return (0, 0);
    }
    // Previous OpenAI flagship profile, then the clean release id some catalogs
    // expose instead of jcode's Sol route. Both still outrank the Claude default.
    if normalized == "gpt-5.6-sol" {
        return (1, 0);
    }
    if normalized == "gpt-5.6" {
        return (1, 1);
    }
    if normalized == claude_default {
        return (2, 0);
    }
    if let Some(position) = crate::provider::ALL_CLAUDE_MODELS
        .iter()
        .position(|candidate| normalize_model_for_preference(candidate) == normalized)
    {
        return (3, position);
    }
    if let Some(position) = crate::provider::ALL_OPENAI_MODELS
        .iter()
        .position(|candidate| normalize_model_for_preference(candidate) == normalized)
    {
        return (4, position);
    }

    // Unknown provider families retain catalog order as the final fallback.
    (5, usize::MAX)
}

/// Curated flagship-first order for Bedrock-hosted models. Bedrock ids carry a
/// vendor prefix (`anthropic.claude-opus-4-...`, `us.anthropic.claude-...`) which
/// `parse_frontier_model`/`normalize_model_for_preference` strip before matching,
/// so the bare canonical ids here line up with the live route ids. Claude Opus
/// is the flagship, then Sonnet, then Nova/Llama/Mistral, then Haiku/cheap.
const ALL_BEDROCK_MODELS: &[&str] = &[
    "claude-opus-4",
    "claude-sonnet-4",
    "claude-3-7-sonnet",
    "claude-3-5-sonnet",
    "amazon.nova-pro",
    "meta.llama3-1-405b-instruct",
    "mistral.mistral-large",
    "claude-3-5-haiku",
    "amazon.nova-lite",
    "amazon.nova-micro",
];

/// Curated flagship-first order for Gemini (Code Assist OAuth + Gemini API).
/// `pro` is Gemini's flagship tier and `flash`/`lite` are the cheaper tiers, so
/// (unlike Claude/OpenAI) `pro` must NOT be treated as a non-flagship marker for
/// this family. Listed newest-and-strongest first.
const ALL_GEMINI_MODELS: &[&str] = &[
    "gemini-3.1-pro",
    "gemini-3-pro",
    "gemini-2.5-pro",
    "gemini-1.5-pro",
    "gemini-3-flash",
    "gemini-2.5-flash",
    "gemini-2.0-flash",
    "gemini-1.5-flash",
];

/// Flagship-first preference tiers used only to break ties when falling back to
/// an arbitrary matching route after a login. Each inner slice is one curated
/// family ordered best-first; earlier families outrank later ones. Returns an
/// empty slice for providers without a curated order (local OpenAI-compatible,
/// raw OpenRouter, ...), which preserves live-catalog order.
///
/// Copilot and Cursor proxy Claude/OpenAI models under their bare canonical ids
/// (`copilot:claude-opus-4-8`), so they share the same "catalog lists the cheap
/// model first" hazard as a direct login and get the combined Claude+OpenAI
/// order. The Claude/OpenAI subscription default bias mirrors jcode's global
/// default model. Bedrock/Azure/Gemini/Antigravity are native hosted catalogs
/// whose route lists are often ordered oldest-first, so they get an explicit
/// curated order too.
fn provider_preferred_model_orders(
    activation: &AuthActivationResult,
) -> &'static [&'static [&'static str]] {
    match activation.provider_id.as_deref() {
        Some("claude") | Some("claude-api") => &[crate::provider::ALL_CLAUDE_MODELS],
        Some("openai") | Some("openai-api") => &[crate::provider::ALL_OPENAI_MODELS],
        Some("copilot") | Some("cursor") => &[
            crate::provider::ALL_CLAUDE_MODELS,
            crate::provider::ALL_OPENAI_MODELS,
        ],
        Some("bedrock") => &[ALL_BEDROCK_MODELS],
        // Azure hosts the OpenAI family.
        Some("azure-openai") => &[crate::provider::ALL_OPENAI_MODELS],
        // Gemini (Code Assist OAuth) and Antigravity both serve Gemini models.
        Some("gemini") | Some("antigravity") => &[ALL_GEMINI_MODELS],
        _ => &[],
    }
}

/// Rank a (possibly date-suffixed) catalog model id against flagship-first
/// preference tiers. Lower is more preferred: an earlier family tier always
/// outranks a later one, and within a tier the curated position decides.
/// Unknown models sort last so they only win when nothing curated matches.
fn preferred_model_rank(orders: &[&[&str]], model: &str) -> usize {
    const TIER_STRIDE: usize = 10_000;
    let normalized = normalize_model_for_preference(model);
    for (tier, order) in orders.iter().enumerate() {
        if let Some(position) = order
            .iter()
            .position(|candidate| normalize_model_for_preference(candidate) == normalized)
        {
            return tier * TIER_STRIDE + position;
        }
    }
    usize::MAX
}

/// Normalize a model id for flagship-preference comparison: lowercase, drop a
/// `[1m]` long-context suffix, strip a trailing 8-digit `-YYYYMMDD` date so live
/// dated ids (`claude-haiku-4-5-20251001`) match bare canonical ids
/// (`claude-haiku-4-5`), and strip hosted-vendor prefixes/suffixes so Bedrock and
/// proxy ids line up with the curated bare ids.
///
/// Examples:
///   `us.anthropic.claude-opus-4-20250514-v1:0` -> `claude-opus-4`
///   `anthropic.claude-3-5-sonnet-20241022-v2:0` -> `claude-3-5-sonnet`
///   `accounts/fireworks/models/qwen3-coder` -> `qwen3-coder`
///   `models/gemini-3-pro-preview` -> `gemini-3-pro`
fn normalize_model_for_preference(model: &str) -> String {
    let mut id = jcode_provider_core::model_id::canonical(model);

    // Drop a `/`-qualified path prefix (`accounts/x/models/y`, `models/gemini`).
    if let Some(idx) = id.rfind('/') {
        id = id[idx + 1..].to_string();
    }

    // Drop a trailing Bedrock version tag (`-v1:0`, `-v2:0`, `:0`).
    if let Some(idx) = id.find(":0") {
        id = id[..idx].to_string();
    }
    if let Some(stripped) = strip_trailing_bedrock_version(&id) {
        id = stripped;
    }

    // Drop a trailing release-date suffix.
    id = jcode_provider_core::model_id::strip_date_suffix(&id).to_string();

    // Drop a trailing `-preview`/`-exp`/`-latest` marketing suffix so Gemini
    // preview ids match their canonical family entry.
    for suffix in ["-preview", "-exp", "-latest"] {
        if let Some(base) = id.strip_suffix(suffix) {
            id = base.to_string();
        }
    }

    // Drop a leading hosted-vendor segment (`anthropic.`, `us.anthropic.`,
    // `meta.`, `amazon.`, `mistral.`) so `anthropic.claude-opus-4` matches the
    // curated `claude-opus-4`. Keep `amazon.nova`/`meta.llama`/`mistral.` whole
    // because those families are listed with their vendor prefix in
    // `ALL_BEDROCK_MODELS`; only strip the region + the redundant `anthropic.`.
    id = strip_bedrock_region_prefix(&id);
    if let Some(rest) = id.strip_prefix("anthropic.") {
        id = rest.to_string();
    }

    id
}

/// Strip a leading Bedrock region routing segment (`us.`, `eu.`, `apac.`,
/// `us-gov.`) from a model id.
fn strip_bedrock_region_prefix(id: &str) -> String {
    for region in ["us-gov.", "us.", "eu.", "apac.", "ap.", "global."] {
        if let Some(rest) = id.strip_prefix(region) {
            return rest.to_string();
        }
    }
    id.to_string()
}

/// Strip a trailing Bedrock version tag like `-v1`, `-v2` (after the `:0` has
/// already been removed). Returns `None` when there is no such tag.
fn strip_trailing_bedrock_version(id: &str) -> Option<String> {
    let (head, tail) = id.rsplit_once('-')?;
    let is_version_tag =
        tail.len() >= 2 && tail.starts_with('v') && tail[1..].chars().all(|c| c.is_ascii_digit());
    is_version_tag.then(|| head.to_string())
}

/// A parsed "frontier flagship" model id: its family prefix (e.g. `claude-opus`
/// or `gpt`) plus an ordered version vector parsed from the trailing numeric
/// components, used to compare releases within a family.
///
/// Only *pure* flagship ids parse successfully: ids carrying a cheaper or
/// specialized tier word (`mini`, `nano`, `haiku`, `flash`, `codex`, `pro`,
/// `chat`, ...) are rejected so a new cheap/specialized model never auto-promotes
/// over the flagship.
struct FrontierModel {
    family: String,
    version: Vec<u64>,
}

/// Tier/specialization words that disqualify an id from frontier auto-promotion
/// for the default (Claude/OpenAI-style) families. These mark cheaper or
/// non-default variants that must never outrank a clean flagship id purely
/// because they share a (possibly higher) version number.
///
/// NOTE: this list is used for families whose flagship id is *bare* (no tier
/// word), e.g. `claude-opus-4-8`, `gpt-5.5`. Families like Gemini, whose flagship
/// id contains a tier word (`gemini-3-pro`), use a family-specific rule instead
/// (see [`frontier_families`]).
const NON_FLAGSHIP_TIER_WORDS: &[&str] = &[
    "mini", "nano", "haiku", "flash", "lite", "small", "tiny", "instant", "codex", "pro", "chat",
    "audio", "realtime", "image", "tts", "embed", "search", "guard", "deep", "thinking",
];

/// One frontier-eligible family: the canonical id prefix to match (after
/// [`normalize_model_for_preference`]) and, optionally, a required flagship tier
/// token that must be present (e.g. Gemini's `pro`). When `flagship_token` is
/// `Some`, version parsing ignores that token and rejects any *other* trailing
/// word; when `None`, the id must be bare (Claude/OpenAI style) and is rejected
/// if it contains any [`NON_FLAGSHIP_TIER_WORDS`].
#[derive(Clone, Copy)]
struct FrontierFamily {
    prefix: &'static str,
    flagship_token: Option<&'static str>,
}

/// Family descriptors eligible for frontier auto-promotion per provider. Only the
/// strongest family per provider is listed (Claude Opus, GPT base, Gemini Pro) so
/// we never auto-promote a cheaper family/tier over the curated flagship.
fn frontier_families(activation: &AuthActivationResult) -> &'static [FrontierFamily] {
    const FABLE: FrontierFamily = FrontierFamily {
        prefix: "claude-fable",
        flagship_token: None,
    };
    const CLAUDE: FrontierFamily = FrontierFamily {
        prefix: "claude-opus",
        flagship_token: None,
    };
    const GPT: FrontierFamily = FrontierFamily {
        prefix: "gpt",
        flagship_token: None,
    };
    const GEMINI: FrontierFamily = FrontierFamily {
        prefix: "gemini",
        flagship_token: Some("pro"),
    };
    match activation.provider_id.as_deref() {
        Some("claude") | Some("claude-api") => &[CLAUDE, FABLE],
        Some("openai") | Some("openai-api") | Some("azure-openai") => &[GPT],
        // Copilot/Cursor proxy both families under canonical ids.
        Some("copilot") | Some("cursor") => &[CLAUDE, FABLE, GPT],
        // Bedrock hosts Claude under `anthropic.claude-opus-...` (prefix stripped
        // by normalize), so the Claude family applies.
        Some("bedrock") => &[CLAUDE],
        Some("gemini") | Some("antigravity") => &[GEMINI],
        _ => &[],
    }
}

/// Parse a model id into a [`FrontierModel`] if it is a clean flagship id for one
/// of `families`. Returns `None` for non-matching families, ids with a
/// non-flagship tier word, or ids without a parseable version.
fn parse_frontier_model(model: &str, families: &[FrontierFamily]) -> Option<FrontierModel> {
    let normalized = normalize_model_for_preference(model);
    // Find the family this id belongs to (longest prefix wins so `claude-opus`
    // is preferred over a hypothetical `claude`).
    let family = families
        .iter()
        .filter(|fam| normalized.starts_with(fam.prefix))
        .max_by_key(|fam| fam.prefix.len())?;

    let version = parse_frontier_version_for_family(&normalized, *family, false)?;
    Some(FrontierModel {
        family: family.prefix.to_string(),
        version,
    })
}

/// Parse a model version within one known frontier family. Curated defaults may
/// carry a quality-profile suffix such as `gpt-5.6-sol`; when `allow_suffix` is
/// true, the leading numeric release is still used as the promotion baseline.
/// Live promotion candidates remain strict, so cheap/specialized variants never
/// self-promote merely because they contain a higher version number.
fn parse_frontier_version_for_family(
    normalized_model: &str,
    family: FrontierFamily,
    allow_suffix: bool,
) -> Option<Vec<u64>> {
    if !normalized_model.starts_with(family.prefix) {
        return None;
    }

    match family.flagship_token {
        None => {
            // Bare-flagship family (Claude/OpenAI): reject any tier word, then
            // require a pure-numeric remainder after the prefix.
            if !allow_suffix
                && NON_FLAGSHIP_TIER_WORDS
                    .iter()
                    .any(|word| normalized_model.contains(word))
            {
                return None;
            }
            let remainder = normalized_model[family.prefix.len()..].trim_matches(['-', '.', ' ']);
            if !allow_suffix {
                return parse_version_components(remainder);
            }
            let mut version = Vec::new();
            for part in remainder.split(['.', '-']) {
                if part.is_empty() {
                    continue;
                }
                match part.parse::<u64>() {
                    Ok(number) => version.push(number),
                    Err(_) if !version.is_empty() => break,
                    Err(_) => return None,
                }
            }
            (!version.is_empty()).then_some(version)
        }
        Some(flagship_token) => {
            // Flagship-token family (Gemini): the id must contain the flagship
            // token and nothing else but version numbers + that token. Reject any
            // other word (e.g. `flash`, `lite`).
            let remainder = normalized_model[family.prefix.len()..].trim_matches(['-', '.', ' ']);
            if remainder.is_empty() {
                return None;
            }
            let mut version = Vec::new();
            let mut saw_flagship_token = false;
            for part in remainder.split(['.', '-']) {
                if part.is_empty() {
                    continue;
                }
                if part == flagship_token {
                    saw_flagship_token = true;
                    continue;
                }
                let number: u64 = part.parse().ok()?; // any other word => reject
                version.push(number);
            }
            if !saw_flagship_token || version.is_empty() {
                return None;
            }
            Some(version)
        }
    }
}

/// Parse a dash/dot-separated numeric version remainder (`4-8`, `5.5`) into a
/// component vector. Returns `None` if any component is non-numeric or empty.
fn parse_version_components(remainder: &str) -> Option<Vec<u64>> {
    if remainder.is_empty() {
        return None;
    }
    let mut version = Vec::new();
    for part in remainder.split(['.', '-']) {
        if part.is_empty() {
            continue;
        }
        version.push(part.parse::<u64>().ok()?);
    }
    (!version.is_empty()).then_some(version)
}

/// Compare two version vectors component-wise (semver-like). Missing trailing
/// components are treated as 0 so `[5]` < `[5, 1]`.
fn version_cmp(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let len = a.len().max(b.len());
    for i in 0..len {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        match av.cmp(&bv) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// Auto-detect a brand-new frontier flagship release among `routes` that is
/// strictly newer than the curated baseline flagship for the same family, and is
/// not already the curated #1. Returns the chosen model id, or `None` when the
/// curated default is still the newest known flagship.
///
/// This is the new-release robustness layer: the day Anthropic ships
/// `claude-opus-4-9` or OpenAI ships `gpt-5.6`, the live catalog will carry it
/// and it auto-promotes to the post-login default without a code change, while
/// cheaper/specialized variants are excluded by [`parse_frontier_model`].
fn newest_frontier_release(
    activation: &AuthActivationResult,
    routes: &[&ModelRoute],
) -> Option<String> {
    let families = frontier_families(activation);
    if families.is_empty() {
        return None;
    }

    // Only auto-promote within the strongest frontier family that is actually
    // present. This prevents a newer lower-priority Opus id from displacing an
    // available Fable default, while still allowing Opus promotion when Fable is
    // absent and GPT promotion when Sol is the active quality profile.
    let active_family = families.iter().copied().find(|family| {
        routes.iter().any(|route| {
            let normalized = normalize_model_for_preference(&route.model);
            parse_frontier_version_for_family(&normalized, *family, true).is_some()
        })
    })?;

    // Baseline: the strongest *available* curated release in the active family.
    // If the preferred profile (for example Sol 5.6) is unavailable, a clean
    // model at that release may still promote over the next available fallback
    // (for example GPT 5.5). When no curated route is available at all, fall back
    // to the shipped catalog baseline so an old unknown id cannot self-promote.
    let curated_baseline = |family: FrontierFamily| -> Option<Vec<u64>> {
        let orders = provider_preferred_model_orders(activation);
        let available_baseline = routes
            .iter()
            .filter(|route| preferred_model_rank(orders, &route.model) != usize::MAX)
            .filter_map(|route| {
                let normalized = normalize_model_for_preference(&route.model);
                parse_frontier_version_for_family(&normalized, family, true)
            })
            .max_by(|a, b| version_cmp(a, b));
        available_baseline.or_else(|| {
            orders
                .iter()
                .flat_map(|order| order.iter())
                .filter_map(|id| {
                    let normalized = normalize_model_for_preference(id);
                    parse_frontier_version_for_family(&normalized, family, true)
                })
                .max_by(|a, b| version_cmp(a, b))
        })
    };

    let mut best: Option<(FrontierModel, String)> = None;
    for route in routes {
        let Some(parsed) = parse_frontier_model(&route.model, families) else {
            continue;
        };
        if parsed.family != active_family.prefix {
            continue;
        }
        // Must strictly beat the curated baseline for its family.
        let Some(baseline) = curated_baseline(active_family) else {
            continue;
        };
        if version_cmp(&parsed.version, &baseline) != std::cmp::Ordering::Greater {
            continue;
        }
        // Among qualifying releases, keep the highest version (preferring the
        // strongest family by the order in `families` on ties).
        let is_better = match &best {
            None => true,
            Some((current, _)) => match version_cmp(&parsed.version, &current.version) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => {
                    // Tie on version: prefer the family listed earlier (stronger).
                    let rank = |fam: &str| {
                        families
                            .iter()
                            .position(|f| f.prefix == fam)
                            .unwrap_or(usize::MAX)
                    };
                    rank(&parsed.family) < rank(&current.family)
                }
                std::cmp::Ordering::Less => false,
            },
        };
        if is_better {
            best = Some((parsed, route.model.clone()));
        }
    }

    best.map(|(_, model)| model)
}

fn route_matches_activation(route: &ModelRoute, activation: &AuthActivationResult) -> bool {
    let api_method = route.api_method_kind();
    let Some(provider_id) = activation.provider_id.as_deref() else {
        if let Some(label) = activation.provider_label.as_deref()
            && route.provider.eq_ignore_ascii_case(label)
        {
            return true;
        }
        return false;
    };

    if api_method.matches_openai_compatible_profile(provider_id) {
        return true;
    }

    if route.api_method.eq_ignore_ascii_case(provider_id) {
        return true;
    }

    match provider_id {
        "claude" => {
            return matches!(
                api_method,
                crate::provider::ModelRouteApiMethod::ClaudeOAuth
            );
        }
        "claude-api" => {
            return route.provider.eq_ignore_ascii_case("Anthropic")
                && matches!(
                    api_method,
                    crate::provider::ModelRouteApiMethod::AnthropicApiKey
                );
        }
        "openai" => {
            return matches!(
                api_method,
                crate::provider::ModelRouteApiMethod::OpenAIOAuth
            );
        }
        "openai-api" => {
            return matches!(
                api_method,
                crate::provider::ModelRouteApiMethod::OpenAIApiKey
            );
        }
        "gemini" => {
            // Gemini's Code Assist OAuth routes carry the `code-assist-oauth`
            // api_method (not the bare provider id), so match on the parsed kind
            // like the other native credential routes above.
            return matches!(
                api_method,
                crate::provider::ModelRouteApiMethod::CodeAssistOAuth
            );
        }
        "jcode" => {
            // Jcode subscription routes deliberately keep their managed public
            // identity even though the runtime reuses OpenRouter transport code.
            return matches!(
                api_method,
                crate::provider::ModelRouteApiMethod::JcodeSubscription
            );
        }
        "azure-openai" => {
            // Azure OpenAI reuses the OpenRouter transport (configured via Azure
            // env), so its routes carry the `openrouter` api_method while keeping
            // the `azure-openai` runtime identity.
            return matches!(api_method, crate::provider::ModelRouteApiMethod::OpenRouter);
        }
        _ => {}
    }

    // OpenAI-compatible auth has a concrete catalog namespace. Accepting a
    // matching display label or generic `openai-compatible` route as success can
    // hide stale/mixed catalogs, especially when providers share model IDs.
    // Legacy/local TUI auth notifications only carry the provider label, so
    // derive this constraint from the normalized profile id as well as explicit
    // protocol metadata.
    if crate::provider_catalog::openai_compatible_profile_by_id(provider_id).is_some()
        || activation.expected_runtime.as_deref() == Some("openai-compatible")
        || activation.expected_catalog_namespace.is_some()
    {
        return false;
    }

    if let Some(label) = activation.provider_label.as_deref()
        && route.provider.eq_ignore_ascii_case(label)
    {
        return true;
    }

    false
}

pub fn normalized_auth_provider_id(provider_hint: Option<&str>) -> Option<&'static str> {
    let provider = provider_hint?.trim();
    if provider.eq_ignore_ascii_case("azure")
        || provider.eq_ignore_ascii_case("azure-openai")
        || provider.eq_ignore_ascii_case("azure openai")
    {
        Some("azure-openai")
    } else if let Some(profile) =
        crate::provider_catalog::resolve_openai_compatible_profile_selection(provider)
    {
        Some(profile.id)
    } else if let Some(descriptor) = crate::provider_catalog::resolve_login_provider(provider) {
        normalized_login_provider_id(descriptor.id)
    } else {
        None
    }
}

/// Pick the preferred first-run provider when the machine already has working
/// OpenAI and/or Anthropic credentials. Keep this aligned with
/// `jcode_provider_core::auto_default_provider`: Claude wins when both families
/// are available, and OAuth wins over an API key within the same family.
///
/// Returning the auth-specific provider id (`openai` vs `openai-api`, `claude`
/// vs `claude-api`) matters because post-login activation uses it to select the
/// matching route and its strongest available model.
pub fn preferred_frontier_auth_provider(status: &crate::auth::AuthStatus) -> Option<&'static str> {
    use crate::auth::AuthState;

    if status.anthropic.state == AuthState::Available {
        if status.anthropic.has_oauth && status.anthropic.oauth_state == AuthState::Available {
            return Some("claude");
        }
        if status.anthropic.has_api_key {
            return Some("claude-api");
        }
        return Some("claude");
    }

    if status.openai == AuthState::Available {
        if status.openai_has_oauth && status.openai_oauth_state == AuthState::Available {
            return Some("openai");
        }
        if status.openai_has_api_key {
            return Some("openai-api");
        }
        // Preserve compatibility with older/partial status snapshots that only
        // populated the aggregate state.
        return Some("openai");
    }

    None
}

fn normalized_login_provider_id(provider_id: &str) -> Option<&'static str> {
    match provider_id.trim().to_ascii_lowercase().as_str() {
        "claude" | "anthropic" => Some("claude"),
        "anthropic-api" | "claude-api" | "anthropic-key" | "claude-key" => Some("claude-api"),
        "openai" => Some("openai"),
        "openai-api" | "openai-key" | "openai-apikey" | "openai-platform" | "platform-openai" => {
            Some("openai-api")
        }
        "openrouter" => Some("openrouter"),
        "jcode" | "subscription" | "jcode-subscription" => Some("jcode"),
        "bedrock" | "aws-bedrock" | "aws_bedrock" => Some("bedrock"),
        "cursor" => Some("cursor"),
        "copilot" => Some("copilot"),
        "gemini" => Some("gemini"),
        "antigravity" => Some("antigravity"),
        _ => None,
    }
}

pub fn provider_display_label(provider_id: Option<&str>) -> Option<String> {
    let provider = normalized_auth_provider_id(provider_id)?;
    if provider == "azure-openai" {
        return Some("Azure OpenAI".to_string());
    }
    crate::provider_catalog::openai_compatible_profile_by_id(provider)
        .map(|profile| profile.display_name.to_string())
        .or_else(|| {
            crate::provider_catalog::resolve_login_provider(provider)
                .map(|descriptor| descriptor.display_name.to_string())
        })
        .or_else(|| Some(provider.to_string()))
}

pub fn activate_auth_change(request: &AuthActivationRequest) -> AuthActivationResult {
    let provider_id = request.provider_id();
    sync_process_env_from_saved_credentials(request, provider_id.as_deref());
    // The notification handler may have probed auth while the newly saved
    // credential was not yet reflected in the process environment. Discard
    // that snapshot after activation so catalog rebuilding sees the new auth.
    super::AuthStatus::invalidate_cache();
    let provider_label = provider_display_label(provider_id.as_deref());
    let activated_model = apply_auth_provider_runtime(provider_id.as_deref());
    AuthActivationResult {
        provider_id,
        provider_label,
        activated_model,
        expected_runtime: request
            .expected_runtime()
            .map(|runtime| runtime.as_str().to_string()),
        expected_catalog_namespace: request
            .expected_catalog_namespace()
            .map(|namespace| namespace.as_str().to_string()),
    }
}

/// Env keys and env-file names that persist API-key credentials for a
/// normalized auth provider id. Empty for OAuth/CLI providers whose
/// credentials live in token stores, not env vars.
fn api_key_env_bindings_for_provider(provider_id: &str) -> Vec<(String, String)> {
    match provider_id {
        "claude-api" => vec![("ANTHROPIC_API_KEY".to_string(), "anthropic.env".to_string())],
        "openai-api" => vec![("OPENAI_API_KEY".to_string(), "openai.env".to_string())],
        "openrouter" => vec![(
            "OPENROUTER_API_KEY".to_string(),
            "openrouter.env".to_string(),
        )],
        "jcode" => vec![
            (
                crate::subscription_catalog::JCODE_API_KEY_ENV.to_string(),
                crate::subscription_catalog::JCODE_ENV_FILE.to_string(),
            ),
            (
                crate::subscription_catalog::JCODE_API_BASE_ENV.to_string(),
                crate::subscription_catalog::JCODE_ENV_FILE.to_string(),
            ),
        ],
        "bedrock" => vec![
            (
                crate::provider::bedrock::API_KEY_ENV.to_string(),
                crate::provider::bedrock::ENV_FILE.to_string(),
            ),
            (
                crate::provider::bedrock::REGION_ENV.to_string(),
                crate::provider::bedrock::ENV_FILE.to_string(),
            ),
        ],
        "cursor" => vec![("CURSOR_API_KEY".to_string(), "cursor.env".to_string())],
        "gemini" => super::gemini::GEMINI_API_KEY_ENV_VARS
            .iter()
            .map(|env_key| {
                (
                    env_key.to_string(),
                    super::gemini::GEMINI_API_KEY_ENV_FILE.to_string(),
                )
            })
            .collect(),
        "azure-openai" => vec![
            (
                super::azure::API_KEY_ENV.to_string(),
                super::azure::ENV_FILE.to_string(),
            ),
            (
                super::azure::ENDPOINT_ENV.to_string(),
                super::azure::ENV_FILE.to_string(),
            ),
            (
                super::azure::MODEL_ENV.to_string(),
                super::azure::ENV_FILE.to_string(),
            ),
        ],
        other => crate::provider_catalog::openai_compatible_profile_by_id(other)
            .map(|profile| {
                let resolved = crate::provider_catalog::resolve_openai_compatible_profile(profile);
                vec![(resolved.api_key_env, resolved.env_file)]
            })
            .unwrap_or_default(),
    }
}

/// Make freshly saved credentials win over stale env vars inherited by this
/// process (issue #453).
///
/// `/login` persists API keys to the per-provider env file under the jcode
/// config dir, but credential resolution
/// ([`crate::provider_catalog::load_api_key_from_env_or_config`]) prefers the
/// process env var. A long-lived server that inherited a stale
/// `ANTHROPIC_API_KEY` (or similar) would therefore keep 401-ing forever even
/// though the login succeeded and the file holds a valid key. On an explicit
/// auth change, overwrite this process's env var with the env-file value when
/// the two diverge, so the just-saved credential is actually used.
fn sync_process_env_from_saved_credentials(
    request: &AuthActivationRequest,
    provider_id: Option<&str>,
) {
    let Some(provider_id) = provider_id else {
        return;
    };
    // Only do this for auth changes that plausibly wrote an env file. OAuth
    // logins do not touch API-key env files, and process-env-preseeded auth
    // means the env var itself is the intended source of truth.
    let explicit_env_file_login = match request.auth.as_ref() {
        Some(auth) => {
            matches!(
                auth.credential_source,
                Some(crate::protocol::AuthCredentialSource::ApiKeyFile) | None
            ) && !matches!(
                auth.auth_method,
                Some(crate::protocol::AuthMethod::ProcessEnvPreseeded)
                    | Some(crate::protocol::AuthMethod::OAuthBrowser)
                    | Some(crate::protocol::AuthMethod::DeviceCode)
            )
        }
        // Legacy hint-only notifications carry no source metadata; syncing is
        // still the safe default because it only runs when the file has a
        // value that differs from the env var.
        None => true,
    };
    if !explicit_env_file_login {
        return;
    }

    for (env_key, env_file) in api_key_env_bindings_for_provider(provider_id) {
        let Some(file_value) =
            crate::provider_catalog::load_env_value_from_config_file(&env_key, &env_file)
        else {
            continue;
        };
        let env_value = std::env::var(&env_key).ok();
        if env_value.as_deref() == Some(file_value.as_str()) {
            continue;
        }
        let had_stale_env = env_value.is_some();
        crate::env::set_var(&env_key, &file_value);
        crate::logging::auth_event(
            "auth_changed_env_synced_from_file",
            provider_id,
            &[
                ("env_key", env_key.as_str()),
                ("env_file", env_file.as_str()),
                (
                    "replaced",
                    if had_stale_env {
                        "stale_process_env"
                    } else {
                        "unset_process_env"
                    },
                ),
            ],
        );
    }
}

fn apply_auth_provider_runtime(provider_id: Option<&str>) -> Option<String> {
    match normalized_auth_provider_id(provider_id) {
        Some("azure-openai") => match crate::provider::activation::apply_azure_openai_runtime() {
            Ok(model) => model,
            Err(error) => {
                let message = error.to_string();
                crate::logging::auth_event(
                    "auth_changed_runtime_activation_failed",
                    "azure-openai",
                    &[("reason", message.as_str())],
                );
                None
            }
        },
        Some(profile_id)
            if direct_provider_activation(profile_id).is_none()
                && crate::provider_catalog::openai_compatible_profile_by_id(profile_id)
                    .is_some() =>
        {
            let Some(profile) =
                crate::provider_catalog::openai_compatible_profile_by_id(profile_id)
            else {
                crate::logging::auth_event(
                    "auth_changed_runtime_activation_failed",
                    profile_id,
                    &[(
                        "reason",
                        "openai-compatible profile disappeared during activation",
                    )],
                );
                return None;
            };
            crate::provider_catalog::force_apply_openai_compatible_profile_env(Some(profile));
            let default_model =
                crate::provider_catalog::resolve_openai_compatible_profile(profile).default_model;
            if let Err(error) =
                crate::provider::activation::apply_openai_compatible_runtime(default_model.clone())
            {
                let message = error.to_string();
                crate::logging::auth_event(
                    "auth_changed_runtime_activation_failed",
                    profile_id,
                    &[("reason", message.as_str())],
                );
                None
            } else {
                default_model
            }
        }
        Some(provider_id) => {
            if let Some(activation) = direct_provider_activation(provider_id)
                && let Err(error) = activation.apply_env()
            {
                let message = error.to_string();
                crate::logging::auth_event(
                    "auth_changed_runtime_activation_failed",
                    provider_id,
                    &[("reason", message.as_str())],
                );
            }
            None
        }
        _ => None,
    }
}

fn direct_provider_activation(provider_id: &str) -> Option<ProviderActivation> {
    let (runtime_id, active) = match normalized_login_provider_id(provider_id)? {
        "claude" => (RuntimeProviderId::Claude, ActiveProvider::Claude),
        "claude-api" => (RuntimeProviderId::ClaudeApiKey, ActiveProvider::Claude),
        "openai" => (RuntimeProviderId::OpenAi, ActiveProvider::OpenAI),
        "openai-api" => (RuntimeProviderId::OpenAiApiKey, ActiveProvider::OpenAI),
        "openrouter" => (RuntimeProviderId::OpenRouter, ActiveProvider::OpenRouter),
        "jcode" => (RuntimeProviderId::Jcode, ActiveProvider::OpenRouter),
        "bedrock" => (RuntimeProviderId::Bedrock, ActiveProvider::Bedrock),
        "cursor" => (RuntimeProviderId::Cursor, ActiveProvider::Cursor),
        "copilot" => (RuntimeProviderId::Copilot, ActiveProvider::Copilot),
        "gemini" => (RuntimeProviderId::Gemini, ActiveProvider::Gemini),
        "antigravity" => (RuntimeProviderId::Antigravity, ActiveProvider::Antigravity),
        "grok-build" => (RuntimeProviderId::GrokBuild, ActiveProvider::OpenRouter),
        _ => return None,
    };
    Some(ProviderActivation::initial(runtime_id, active))
}

pub fn model_switch_request_for_provider_id(
    provider_id: Option<&str>,
    _provider_name: &str,
    model: &str,
) -> String {
    match normalized_auth_provider_id(provider_id) {
        Some("azure-openai") => format!("openrouter:{}", model),
        Some(profile_id)
            if profile_id != "azure-openai"
                && crate::provider_catalog::openai_compatible_profile_by_id(profile_id)
                    .is_some() =>
        {
            format!("{}:{}", profile_id, model)
        }
        Some("claude") => format!("claude-oauth:{}", model),
        Some("claude-api") => format!("claude-api:{}", model),
        Some("openai") => format!("openai-oauth:{}", model),
        Some("openai-api") => format!("openai-api:{}", model),
        Some("openrouter") => format!("openrouter:{}", model),
        Some("jcode") => model.to_string(),
        Some("bedrock") => format!("bedrock:{}", model),
        Some("cursor") => format!("cursor:{}", model),
        Some("copilot") => format!("copilot:{}", model),
        Some("gemini") => format!("gemini:{}", model),
        Some("antigravity") => format!("antigravity:{}", model),
        _ => model.to_string(),
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
