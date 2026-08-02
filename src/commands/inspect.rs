//! Interactive inspection command.

use std::io::{self, Write};

use crate::{
    application, baseline, git, inspection, output, provider, provider::ModelProvider, storage,
};

pub(crate) fn run(project_id: &str) -> anyhow::Result<()> {
    let data_paths = storage::default_data_paths()?;
    let id = super::resolve_project_id(&data_paths, project_id)?;
    let project = storage::project_by_id(&data_paths, id)?
        .ok_or_else(|| anyhow::anyhow!("project {project_id} is not registered"))?;
    let reporter = output::InspectionReporter::new();
    let validated_repository = git::validate_repository_for_inspection(&project.canonical_path)?;
    let inspected_metadata = git::metadata(&validated_repository.root)?;
    let status = git::working_tree_status(&validated_repository.root)?;
    let baseline = storage::latest_baseline(&data_paths, id)?;
    let inspection_status =
        baseline::status_since_baseline(&project.canonical_path, &status, baseline.as_ref())?;
    let retryable_attempt = storage::failed_provider_attempt_for_project(&data_paths, id)?;
    let reused_attempt = retryable_attempt.is_some();
    let initial_inspection = project.lifecycle_state == "registered_needs_inspection";
    let branch = inspected_metadata
        .branch
        .as_deref()
        .unwrap_or("detached HEAD");
    let head_commit = inspected_metadata
        .head_commit
        .as_deref()
        .unwrap_or("unborn");
    reporter.stage(
        1,
        "Repository check",
        output::Style::Success,
        format!("{branch} @ {head_commit}"),
    );
    reporter.detail("project", &project.name);
    reporter.detail(
        "changed paths",
        baseline::changed_path_count(&inspection_status),
    );
    match &retryable_attempt {
        Some(attempt) => {
            println!(
                "inspection scope for {}: retained approved evidence from attempt {}",
                project.name, attempt.id
            );
            println!(
                "current changed paths detected: {}",
                baseline::changed_path_count(&inspection_status)
            );
            for file in &attempt.bundle.files {
                println!("scope\t{}", file.path);
            }
            print!("Retry the retained approved evidence bundle? [y/N] ");
        }
        None => {
            let scope_description = if initial_inspection {
                "initial repository context"
            } else {
                "changed files and direct structural context"
            };
            println!("inspection scope for {}: {scope_description}", project.name);
            println!(
                "changed paths detected: {}",
                baseline::changed_path_count(&inspection_status)
            );
            for path in baseline::inspection_scope_paths(&inspection_status) {
                println!("scope\t{path}");
            }
            print!("Inspect the reported repository files? [y/N] ");
        }
    }
    io::stdout().flush()?;
    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    if !matches!(response.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        reporter.stage(
            2,
            "Evidence preparation",
            output::Style::Warning,
            "not started",
        );
        println!("inspection cancelled; no repository content was read");
        return Ok(());
    }

    let (bundle, attempt_id, provider_repository) = if let Some(attempt) = retryable_attempt {
        storage::retry_provider_attempt(&data_paths, attempt.id)?;
        let provider_repository =
            git::validate_repository_for_inspection(&validated_repository.root)?;
        (attempt.bundle, attempt.id, provider_repository)
    } else {
        let package = if initial_inspection {
            inspection::build_initial_package(
                &validated_repository,
                project_id,
                &inspection_status,
            )?
        } else {
            inspection::build_incremental_package(
                &validated_repository,
                project_id,
                &inspection_status,
            )?
        };
        let post_inspection_repository =
            git::validate_repository_for_inspection(&validated_repository.root)?;
        if post_inspection_repository.root != validated_repository.root
            || post_inspection_repository.head_commit != validated_repository.head_commit
        {
            anyhow::bail!(
                "repository changed during inspection; no evidence was staged, retry the inspection"
            );
        }
        let bundle_json = serde_json::to_string(&package.bundle)?;
        let working_tree_status_json = baseline::baseline_working_tree_status(&status)?;
        let uncommitted_fingerprints_json =
            baseline::uncommitted_evidence_fingerprints(&project.canonical_path, &status)?;
        let attempt_id = storage::stage_inspection_attempt_with_baseline_provenance(
            &data_paths,
            id,
            package.bundle.schema_version,
            &bundle_json,
            Some(&package.code_map_json),
            &storage::BaselineProvenance {
                repository_commit: Some(validated_repository.head_commit.clone()),
                repository_branch: inspected_metadata.branch.clone(),
                working_tree_status_json,
                uncommitted_fingerprints_json,
            },
        )?;
        (package.bundle, attempt_id, post_inspection_repository)
    };
    let bundle_bytes = serde_json::to_vec(&bundle)?;
    let redaction_count = bundle.files.iter().filter(|file| file.redacted).count();
    let evidence_detail = if reused_attempt {
        format!(
            "reused approved bundle: {} files, {} KB, {redaction_count} redactions",
            bundle.files.len(),
            bundle_bytes.len().div_ceil(1024)
        )
    } else {
        format!(
            "{} files, {} KB, {redaction_count} redactions",
            bundle.files.len(),
            bundle_bytes.len().div_ceil(1024)
        )
    };
    reporter.stage(
        2,
        "Evidence preparation",
        output::Style::Success,
        evidence_detail,
    );
    if !bundle.initial_inspection && !reused_attempt {
        reporter.detail(
            "current repository changes",
            baseline::changed_path_count(&inspection_status),
        );
    }
    reporter.stage(
        3,
        "Local staging",
        output::Style::Success,
        format!("attempt {attempt_id}"),
    );
    let provider = match provider::OpenRouterProvider::from_environment() {
        Ok(provider) => provider,
        Err(error) => {
            reporter.stage(
                4,
                "OpenRouter request",
                output::Style::Failure,
                error.to_string(),
            );
            let metadata =
                provider::ProviderInvocationMetadata::new("openrouter", "unconfigured", 1)?;
            storage::record_provider_failure(
                &data_paths,
                attempt_id,
                &metadata,
                error
                    .failure_category()
                    .unwrap_or(provider::ProviderFailureCategory::Configuration),
            )?;
            return Err(error.into());
        }
    };
    let configured_metadata = provider.metadata();
    reporter.stage(
        4,
        "OpenRouter request",
        output::Style::Info,
        format!(
            "provider={} model={}",
            configured_metadata.provider_id, configured_metadata.model_id
        ),
    );
    reporter.waiting("waiting for model response...");
    let result = application::submit_staged_bundle(
        &data_paths,
        attempt_id,
        &provider_repository,
        &bundle,
        &provider,
        |response, metadata| {
            reporter.stage(
                5,
                "Response validation",
                output::Style::Success,
                format!(
                    "{} proposals, {} questions",
                    response.proposals.len(),
                    response.questions.len()
                ),
            );
            reporter.detail("routed model", &metadata.model_id);
        },
    );
    if let Err(error) = result {
        let stage = error
            .downcast_ref::<provider::ProviderError>()
            .map_or(6, |_| 5);
        let label = if stage == 5 {
            "Response validation"
        } else {
            "SQLite storage"
        };
        reporter.stage(stage, label, output::Style::Failure, error.to_string());
        return Err(error);
    }
    reporter.stage(
        6,
        "SQLite storage",
        output::Style::Success,
        "pending review",
    );
    reporter.stage(
        7,
        "Inspection complete",
        output::Style::Success,
        format!("run: pmemc review {}", project.name),
    );
    Ok(())
}
