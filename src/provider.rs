//! Narrow, validated model-provider boundary for inspection proposals.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::inspection::EvidenceBundle;

const RESPONSE_SCHEMA_VERSION: u8 = 1;

/// A proposal lifecycle claimed by the provider and validated before storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposedLifecycle {
    Committed,
    InProgress,
}

/// Provider-supplied confidence retained as pending-review metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposedConfidence {
    Exact,
    Inferred,
    UserConfirmed,
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
    /// Return untrusted, schema-validated proposals for an approved bundle.
    fn propose(&self, bundle: &EvidenceBundle) -> Result<ProviderResponse, ProviderError>;
}

/// A deterministic adapter for offline core tests.
#[derive(Debug, Clone)]
pub struct FakeProvider {
    response: ProviderResponse,
}

impl FakeProvider {
    /// Construct a fake adapter with one response returned on every invocation.
    #[must_use]
    pub fn new(response: ProviderResponse) -> Self {
        Self { response }
    }
}

impl ModelProvider for FakeProvider {
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

fn validate_response(
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
