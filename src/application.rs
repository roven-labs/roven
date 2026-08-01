//! Application workflows that coordinate providers and durable inspection attempts.

use crate::{inspection, provider, storage};

/// Stage an approved bundle, invoke a supplied provider, and retain only
/// pending-review output. Tests supply [`provider::FakeProvider`] here so this
/// full path never needs network access.
///
/// # Errors
///
/// Returns an error after retaining a retryable attempt when provider work
/// fails, or when local persistence cannot complete atomically.
pub fn submit_approved_bundle(
    data_paths: &storage::DataPaths,
    project_id: i64,
    bundle: &inspection::EvidenceBundle,
    provider: &dyn provider::ModelProvider,
) -> anyhow::Result<i64> {
    let bundle_json = serde_json::to_string(bundle)?;
    let attempt_id = storage::stage_inspection_attempt(
        data_paths,
        project_id,
        bundle.schema_version,
        &bundle_json,
    )?;
    submit_staged_bundle(data_paths, attempt_id, bundle, provider, |_, _| {})?;
    Ok(attempt_id)
}

pub(crate) fn submit_staged_bundle(
    data_paths: &storage::DataPaths,
    attempt_id: i64,
    bundle: &inspection::EvidenceBundle,
    provider: &dyn provider::ModelProvider,
    on_valid_response: impl FnOnce(&provider::ProviderResponse, &provider::ProviderInvocationMetadata),
) -> anyhow::Result<()> {
    let failure_metadata = provider.metadata();
    let response = match provider.propose(bundle) {
        Ok(response) => response,
        Err(error) => {
            storage::record_provider_failure(
                data_paths,
                attempt_id,
                &failure_metadata,
                error
                    .failure_category()
                    .unwrap_or(provider::ProviderFailureCategory::RequestFailed),
            )?;
            return Err(error.into());
        }
    };
    let metadata = provider.metadata();
    on_valid_response(&response, &metadata);
    storage::store_provider_response(data_paths, attempt_id, &metadata, &response)?;
    Ok(())
}
