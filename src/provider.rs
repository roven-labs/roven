//! Narrow, validated model-provider boundary for inspection proposals.

use std::{collections::BTreeSet, env, fmt, thread, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::inspection::EvidenceBundle;

const RESPONSE_SCHEMA_VERSION: u8 = 1;
const OPENROUTER_CHAT_COMPLETIONS_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_MAX_ATTEMPTS: u8 = 3;
const MAX_ATTEMPTS: u8 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(100);

/// Non-secret metadata recorded for each provider invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInvocationMetadata {
    /// Stable adapter identifier, such as `openrouter`.
    pub provider_id: String,
    /// Configured provider model identifier.
    pub model_id: String,
    /// Version of the prompt contract used for the request.
    pub prompt_schema_version: u8,
}

impl ProviderInvocationMetadata {
    /// Construct metadata after validating values safe to persist and display.
    ///
    /// # Errors
    ///
    /// Returns an error for blank identifiers or a zero prompt schema version.
    pub fn new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        prompt_schema_version: u8,
    ) -> Result<Self, ProviderError> {
        let provider_id = provider_id.into();
        let model_id = model_id.into();
        if provider_id.trim().is_empty() || model_id.trim().is_empty() || prompt_schema_version == 0
        {
            return Err(ProviderError::InvalidConfiguration);
        }
        Ok(Self {
            provider_id,
            model_id,
            prompt_schema_version,
        })
    }
}

/// Safe, non-secret classification recorded when a provider call fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFailureCategory {
    /// Required provider configuration was unavailable or invalid.
    Configuration,
    /// The provider rejected authentication.
    Unauthorized,
    /// The provider rate limited the request after bounded retries.
    RateLimited,
    /// The bounded request deadline elapsed.
    TimedOut,
    /// The provider returned invalid or incomplete structured output.
    InvalidResponse,
    /// A transport or other provider request failed.
    RequestFailed,
    /// The provider had a retryable service failure.
    TemporarilyUnavailable,
}

impl ProviderFailureCategory {
    /// Return the persisted safe category name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Unauthorized => "unauthorized",
            Self::RateLimited => "rate_limited",
            Self::TimedOut => "timed_out",
            Self::InvalidResponse => "invalid_response",
            Self::RequestFailed => "request_failed",
            Self::TemporarilyUnavailable => "temporarily_unavailable",
        }
    }
}

impl fmt::Display for ProviderFailureCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A proposal lifecycle claimed by the provider and validated before storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposedLifecycle {
    Committed,
    InProgress,
}

impl ProposedLifecycle {
    /// Return the persisted lifecycle spelling defined by the response schema.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::InProgress => "in_progress",
        }
    }
}

/// Provider-supplied confidence retained as pending-review metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposedConfidence {
    Exact,
    Inferred,
    UserConfirmed,
}

impl ProposedConfidence {
    /// Return the persisted confidence spelling defined by the response schema.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Inferred => "inferred",
            Self::UserConfirmed => "user_confirmed",
        }
    }
}

/// One untrusted project-fact proposal tied to selected evidence paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub fact_kind: String,
    pub statement: String,
    pub lifecycle: ProposedLifecycle,
    pub confidence: ProposedConfidence,
    pub evidence_paths: Vec<String>,
}

/// Versioned response returned by a model provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderResponse {
    pub schema_version: u8,
    pub proposals: Vec<Proposal>,
    pub questions: Vec<String>,
}

/// Errors from an untrusted provider response or provider invocation.
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider response is invalid; no proposals were stored")]
    InvalidResponse,
    #[error("OPENROUTER_API_KEY is required to inspect a project")]
    MissingApiKey,
    #[error(
        "OpenRouter configuration is invalid; set PMEMC_OPENROUTER_MODEL and valid bounded settings"
    )]
    InvalidConfiguration,
    #[error("OpenRouter request failed ({category})")]
    RequestFailed { category: ProviderFailureCategory },
}

impl ProviderError {
    /// Return a safe category suitable for durable failure metadata.
    #[must_use]
    pub const fn failure_category(&self) -> Option<ProviderFailureCategory> {
        match self {
            Self::InvalidResponse => Some(ProviderFailureCategory::InvalidResponse),
            Self::RequestFailed { category } => Some(*category),
            Self::MissingApiKey | Self::InvalidConfiguration => {
                Some(ProviderFailureCategory::Configuration)
            }
        }
    }
}

/// The replaceable boundary between PMEMC and a model provider.
pub trait ModelProvider {
    /// Return non-secret metadata that must be retained with the response.
    fn metadata(&self) -> ProviderInvocationMetadata;

    /// Return untrusted, schema-validated proposals for an approved bundle.
    fn propose(&self, bundle: &EvidenceBundle) -> Result<ProviderResponse, ProviderError>;
}

/// Configuration for the synchronous OpenRouter adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRouterConfig {
    model_id: String,
    timeout: Duration,
    max_attempts: u8,
}

impl OpenRouterConfig {
    /// Build a bounded OpenRouter configuration.
    ///
    /// # Errors
    ///
    /// Returns an error for a blank model, a zero timeout, or unbounded retry
    /// count.
    pub fn new(
        model_id: impl Into<String>,
        timeout: Duration,
        max_attempts: u8,
    ) -> Result<Self, ProviderError> {
        let model_id = model_id.into();
        if model_id.trim().is_empty()
            || timeout.is_zero()
            || max_attempts == 0
            || max_attempts > MAX_ATTEMPTS
        {
            return Err(ProviderError::InvalidConfiguration);
        }
        Ok(Self {
            model_id,
            timeout,
            max_attempts,
        })
    }

    /// Read non-secret provider configuration from the process environment.
    ///
    /// `PMEMC_OPENROUTER_MODEL` is required. `PMEMC_OPENROUTER_TIMEOUT_SECS`
    /// and `PMEMC_OPENROUTER_MAX_ATTEMPTS` default to bounded values.
    ///
    /// # Errors
    ///
    /// Returns an error when a value is missing or invalid.
    pub fn from_environment() -> Result<Self, ProviderError> {
        Self::from_environment_with(|name| env::var(name).ok())
    }

    /// Read configuration from an injected environment source for tests.
    ///
    /// # Errors
    ///
    /// Returns an error when a value is missing or invalid.
    pub fn from_environment_with(
        value: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ProviderError> {
        let model_id =
            value("PMEMC_OPENROUTER_MODEL").ok_or(ProviderError::InvalidConfiguration)?;
        let timeout_seconds = value("PMEMC_OPENROUTER_TIMEOUT_SECS")
            .map_or(Ok(DEFAULT_TIMEOUT_SECONDS), |value| value.parse::<u64>())
            .map_err(|_| ProviderError::InvalidConfiguration)?;
        let max_attempts = value("PMEMC_OPENROUTER_MAX_ATTEMPTS")
            .map_or(Ok(DEFAULT_MAX_ATTEMPTS), |value| value.parse::<u8>())
            .map_err(|_| ProviderError::InvalidConfiguration)?;
        Self::new(model_id, Duration::from_secs(timeout_seconds), max_attempts)
    }

    /// Return metadata that is safe to persist with proposals.
    #[must_use]
    pub fn metadata(&self) -> ProviderInvocationMetadata {
        ProviderInvocationMetadata {
            provider_id: "openrouter".into(),
            model_id: self.model_id.clone(),
            prompt_schema_version: RESPONSE_SCHEMA_VERSION,
        }
    }
}

/// Synchronous transport boundary allowing provider tests to run without a network.
pub trait OpenRouterTransport {
    /// Send one OpenRouter completion request and return the raw response body.
    fn complete(&self, api_key: &str, request: &Value) -> Result<String, ProviderFailureCategory>;
}

/// Production HTTPS transport for OpenRouter.
pub struct UreqOpenRouterTransport {
    agent: ureq::Agent,
}

impl UreqOpenRouterTransport {
    fn new(timeout: Duration) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .https_only(true)
            .build()
            .new_agent();
        Self { agent }
    }
}

impl OpenRouterTransport for UreqOpenRouterTransport {
    fn complete(&self, api_key: &str, request: &Value) -> Result<String, ProviderFailureCategory> {
        let authorization = format!("Bearer {api_key}");
        let mut response = self
            .agent
            .post(OPENROUTER_CHAT_COMPLETIONS_URL)
            .header("Authorization", &authorization)
            .header("Content-Type", "application/json")
            .send_json(request)
            .map_err(classify_ureq_error)?;
        response
            .body_mut()
            .read_to_string()
            .map_err(|_| ProviderFailureCategory::RequestFailed)
    }
}

/// Production OpenRouter provider, deliberately opaque to avoid key exposure.
pub struct OpenRouterProvider<T = UreqOpenRouterTransport> {
    config: OpenRouterConfig,
    api_key: String,
    transport: T,
}

impl OpenRouterProvider<UreqOpenRouterTransport> {
    /// Construct the production adapter from its documented environment values.
    ///
    /// # Errors
    ///
    /// Returns an error when the key or non-secret configuration is unavailable.
    pub fn from_environment() -> Result<Self, ProviderError> {
        let api_key = env::var("OPENROUTER_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or(ProviderError::MissingApiKey)?;
        let config = OpenRouterConfig::from_environment()?;
        let transport = UreqOpenRouterTransport::new(config.timeout);
        Ok(Self {
            config,
            api_key,
            transport,
        })
    }
}

impl<T> OpenRouterProvider<T> {
    /// Construct an adapter with an injected transport for deterministic tests.
    #[must_use]
    pub fn with_transport(
        config: OpenRouterConfig,
        api_key: impl Into<String>,
        transport: T,
    ) -> Self {
        Self {
            config,
            api_key: api_key.into(),
            transport,
        }
    }
}

impl<T: OpenRouterTransport> ModelProvider for OpenRouterProvider<T> {
    fn metadata(&self) -> ProviderInvocationMetadata {
        self.config.metadata()
    }

    fn propose(&self, bundle: &EvidenceBundle) -> Result<ProviderResponse, ProviderError> {
        let request = openrouter_request(&self.config.model_id, bundle)?;
        for attempt in 0..self.config.max_attempts {
            match self.transport.complete(&self.api_key, &request) {
                Ok(body) => return parse_openrouter_response(&body, bundle),
                Err(category)
                    if matches!(
                        category,
                        ProviderFailureCategory::RateLimited
                            | ProviderFailureCategory::TemporarilyUnavailable
                    ) && attempt + 1 < self.config.max_attempts =>
                {
                    thread::sleep(RETRY_DELAY);
                }
                Err(category) => return Err(ProviderError::RequestFailed { category }),
            }
        }
        Err(ProviderError::RequestFailed {
            category: ProviderFailureCategory::RequestFailed,
        })
    }
}

/// A deterministic adapter for offline core tests.
#[derive(Debug, Clone)]
pub struct FakeProvider {
    response: ProviderResponse,
    metadata: ProviderInvocationMetadata,
}

impl FakeProvider {
    /// Construct a fake adapter with one response returned on every invocation.
    #[must_use]
    pub fn new(response: ProviderResponse) -> Self {
        Self {
            response,
            metadata: ProviderInvocationMetadata {
                provider_id: "fake".into(),
                model_id: "offline-test-model".into(),
                prompt_schema_version: RESPONSE_SCHEMA_VERSION,
            },
        }
    }
}

impl ModelProvider for FakeProvider {
    fn metadata(&self) -> ProviderInvocationMetadata {
        self.metadata.clone()
    }

    fn propose(&self, bundle: &EvidenceBundle) -> Result<ProviderResponse, ProviderError> {
        validate_response(&self.response, bundle)?;
        Ok(self.response.clone())
    }
}

/// Decode and validate a JSON response against an approved evidence bundle.
///
/// # Errors
///
/// Returns an error when JSON or its schema is invalid, or a proposal cites
/// evidence outside the selected bundle.
pub fn parse_response(
    response_json: &str,
    bundle: &EvidenceBundle,
) -> Result<ProviderResponse, ProviderError> {
    let response =
        serde_json::from_str(response_json).map_err(|_| ProviderError::InvalidResponse)?;
    validate_response(&response, bundle)?;
    Ok(response)
}

/// Validate a decoded provider response against the exact approved evidence.
///
/// # Errors
///
/// Returns an error if the response schema or its evidence references are not
/// valid for the supplied bundle.
pub fn validate_response(
    response: &ProviderResponse,
    bundle: &EvidenceBundle,
) -> Result<(), ProviderError> {
    if response.schema_version != RESPONSE_SCHEMA_VERSION {
        return Err(ProviderError::InvalidResponse);
    }
    let selected_paths = bundle
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    for proposal in &response.proposals {
        if !valid_fact_kind(&proposal.fact_kind) {
            return Err(ProviderError::InvalidResponse);
        }
        if proposal.statement.trim().is_empty() {
            return Err(ProviderError::InvalidResponse);
        }
        if proposal.evidence_paths.is_empty()
            || proposal
                .evidence_paths
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != proposal.evidence_paths.len()
            || proposal
                .evidence_paths
                .iter()
                .any(|path| !selected_paths.contains(path.as_str()))
        {
            return Err(ProviderError::InvalidResponse);
        }
    }
    if response
        .questions
        .iter()
        .any(|question| question.trim().is_empty())
    {
        return Err(ProviderError::InvalidResponse);
    }
    Ok(())
}

fn valid_fact_kind(fact_kind: &str) -> bool {
    !fact_kind.is_empty()
        && fact_kind.len() <= 64
        && fact_kind
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn openrouter_request(model_id: &str, bundle: &EvidenceBundle) -> Result<Value, ProviderError> {
    let evidence_json =
        serde_json::to_string(bundle).map_err(|_| ProviderError::InvalidResponse)?;
    let prompt = format!(
        "PMEMC proposal schema version: {RESPONSE_SCHEMA_VERSION}\n\
         Return only one JSON object with exactly schema_version, proposals, and questions.\n\
         Each proposal must have exactly fact_kind, statement, lifecycle, confidence, and evidence_paths.\n\
         fact_kind is a lowercase snake_case identifier of at most 64 characters.\n\
         lifecycle is committed or in_progress; confidence is exact, inferred, or user_confirmed.\n\
         Never infer metrics, ownership, rationale, or claims without selected evidence.\n\
         Put uncertainty in questions. Cite only evidence_paths supplied below.\n\
         Approved evidence bundle:\n{evidence_json}"
    );
    Ok(json!({
        "model": model_id,
        "temperature": 0,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "pmemc_provider_response_v1",
                "strict": true,
                "schema": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["schema_version", "proposals", "questions"],
                    "properties": {
                        "schema_version": {"type": "integer", "const": RESPONSE_SCHEMA_VERSION},
                        "proposals": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["fact_kind", "statement", "lifecycle", "confidence", "evidence_paths"],
                                "properties": {
                                    "fact_kind": {"type": "string", "minLength": 1, "maxLength": 64, "pattern": "^[a-z0-9_]+$"},
                                    "statement": {"type": "string", "minLength": 1},
                                    "lifecycle": {"type": "string", "enum": ["committed", "in_progress"]},
                                    "confidence": {"type": "string", "enum": ["exact", "inferred", "user_confirmed"]},
                                    "evidence_paths": {
                                        "type": "array",
                                        "minItems": 1,
                                        "items": {"type": "string", "minLength": 1}
                                    }
                                }
                            }
                        },
                        "questions": {
                            "type": "array",
                            "items": {"type": "string", "minLength": 1}
                        }
                    }
                }
            }
        },
        "messages": [
            {"role": "system", "content": "You produce evidence-backed PMEMC project-memory proposals."},
            {"role": "user", "content": prompt}
        ]
    }))
}

fn parse_openrouter_response(
    response_body: &str,
    bundle: &EvidenceBundle,
) -> Result<ProviderResponse, ProviderError> {
    let response: Value =
        serde_json::from_str(response_body).map_err(|_| ProviderError::InvalidResponse)?;
    let content = response
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or(ProviderError::InvalidResponse)?;
    parse_response(content, bundle)
}

fn classify_ureq_error(error: ureq::Error) -> ProviderFailureCategory {
    match error {
        ureq::Error::StatusCode(401 | 403) => ProviderFailureCategory::Unauthorized,
        ureq::Error::StatusCode(429) => ProviderFailureCategory::RateLimited,
        ureq::Error::StatusCode(500..=599) => ProviderFailureCategory::TemporarilyUnavailable,
        ureq::Error::Timeout(_) => ProviderFailureCategory::TimedOut,
        _ => ProviderFailureCategory::RequestFailed,
    }
}
