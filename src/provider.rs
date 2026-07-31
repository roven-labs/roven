//! Narrow, validated model-provider boundary for inspection proposals.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::inspection::EvidenceBundle;

const RESPONSE_SCHEMA_VERSION: u8 = 1;

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
            return Err(ProviderError::InvalidResponse {
                message: "provider invocation metadata is incomplete".into(),
            });
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
}

impl ProviderFailureCategory {
    /// Return the persisted safe category name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::RateLimited => "rate_limited",
            Self::TimedOut => "timed_out",
            Self::InvalidResponse => "invalid_response",
            Self::RequestFailed => "request_failed",
        }
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
    #[error("provider response is invalid: {message}")]
    InvalidResponse { message: String },
    #[error("provider request failed: {message}")]
    RequestFailed { message: String },
}

/// The replaceable boundary between PMEMC and a model provider.
pub trait ModelProvider {
    /// Return non-secret metadata that must be retained with the response.
    fn metadata(&self) -> ProviderInvocationMetadata;

    /// Return untrusted, schema-validated proposals for an approved bundle.
    fn propose(&self, bundle: &EvidenceBundle) -> Result<ProviderResponse, ProviderError>;
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
        serde_json::from_str(response_json).map_err(|error| ProviderError::InvalidResponse {
            message: error.to_string(),
        })?;
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
        return Err(ProviderError::InvalidResponse {
            message: format!("unsupported schema version {}", response.schema_version),
        });
    }
    let selected_paths = bundle
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    for proposal in &response.proposals {
        if proposal.statement.trim().is_empty() {
            return Err(ProviderError::InvalidResponse {
                message: "proposal statement is empty".into(),
            });
        }
        if proposal.evidence_paths.is_empty()
            || proposal
                .evidence_paths
                .iter()
                .any(|path| !selected_paths.contains(path.as_str()))
        {
            return Err(ProviderError::InvalidResponse {
                message: "proposal references unselected evidence".into(),
            });
        }
    }
    if response
        .questions
        .iter()
        .any(|question| question.trim().is_empty())
    {
        return Err(ProviderError::InvalidResponse {
            message: "question is empty".into(),
        });
    }
    Ok(())
}
