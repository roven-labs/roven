//! Approved-baseline comparison and working-tree fingerprint policy.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::{git, storage};

pub(crate) fn changed_path_count(status: &git::WorkingTreeStatus) -> usize {
    inspection_scope_paths(status).len()
}

pub(crate) fn inspection_scope_paths(status: &git::WorkingTreeStatus) -> BTreeSet<&str> {
    status
        .committed_paths
        .iter()
        .chain(&status.added_paths)
        .chain(&status.modified_paths)
        .chain(&status.untracked_paths)
        .chain(&status.staged_paths)
        .chain(&status.unstaged_paths)
        .chain(&status.deleted_paths)
        .map(String::as_str)
        .chain(
            status.relationships.iter().flat_map(|relationship| {
                [relationship.source.as_str(), relationship.target.as_str()]
            }),
        )
        .collect()
}

pub(crate) fn baseline_working_tree_status(
    status: &git::WorkingTreeStatus,
) -> anyhow::Result<String> {
    Ok(serde_json::json!({
        "added_paths": status.added_paths,
        "modified_paths": status.modified_paths,
        "untracked_paths": status.untracked_paths,
        "staged_paths": status.staged_paths,
        "unstaged_paths": status.unstaged_paths,
        "deleted_paths": status.deleted_paths,
        "relationships": status.relationships.iter().map(|relationship| serde_json::json!({
            "kind": match relationship.kind { git::PathRelationshipKind::Renamed => "renamed", git::PathRelationshipKind::Copied => "copied" },
            "source": relationship.source,
            "target": relationship.target,
        })).collect::<Vec<_>>(),
    })
    .to_string())
}

pub(crate) fn uncommitted_evidence_fingerprints(
    repository: &std::path::Path,
    status: &git::WorkingTreeStatus,
) -> anyhow::Result<String> {
    let fingerprints = inspection_scope_paths(status)
        .into_iter()
        .filter(|path| !working_tree_path_is_missing(repository, status, path))
        .map(|path| {
            Ok((
                path.to_owned(),
                git::working_tree_fingerprint(repository, path)?,
            ))
        })
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    Ok(serde_json::to_string(&fingerprints)?)
}

pub(crate) fn status_since_baseline(
    repository: &std::path::Path,
    status: &git::WorkingTreeStatus,
    baseline: Option<&storage::InspectionBaseline>,
) -> anyhow::Result<git::WorkingTreeStatus> {
    let Some(baseline) = baseline else {
        return Ok(status.clone());
    };
    let baseline_status: Value = serde_json::from_str(&baseline.working_tree_status_json)?;
    let baseline_paths = json_status_paths(&baseline_status);
    let baseline_untracked = baseline_status
        .get("untracked_paths")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let baseline_fingerprints: BTreeMap<String, String> =
        serde_json::from_str(&baseline.uncommitted_fingerprints_json)?;
    let mut compared_status = status.clone();
    compared_status.committed_paths = baseline
        .repository_commit
        .as_deref()
        .map(|commit| git::committed_paths_since(repository, commit))
        .transpose()?
        .unwrap_or_default();
    let current_paths = inspection_scope_paths(&compared_status)
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let current_fingerprints = current_paths
        .iter()
        .filter(|path| !working_tree_path_is_missing(repository, &compared_status, path))
        .map(|path| {
            Ok((
                path.clone(),
                git::working_tree_fingerprint(repository, path)?,
            ))
        })
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    let mut changed_paths = current_paths
        .iter()
        .filter(|path| {
            !baseline_paths.contains(path.as_str())
                || compared_status.committed_paths.contains(path)
                || working_tree_path_is_missing(repository, &compared_status, path)
                || baseline_fingerprints.get(path.as_str())
                    != current_fingerprints.get(path.as_str())
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    changed_paths.extend(
        baseline_fingerprints
            .keys()
            .filter(|path| {
                baseline_untracked.contains(path.as_str()) && !current_paths.contains(*path)
            })
            .cloned(),
    );
    Ok(filter_status(&compared_status, &changed_paths))
}

fn json_status_paths(status: &Value) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for key in [
        "added_paths",
        "modified_paths",
        "untracked_paths",
        "staged_paths",
        "unstaged_paths",
        "deleted_paths",
    ] {
        if let Some(values) = status.get(key).and_then(Value::as_array) {
            paths.extend(values.iter().filter_map(Value::as_str).map(str::to_owned));
        }
    }
    if let Some(relationships) = status.get("relationships").and_then(Value::as_array) {
        for relationship in relationships {
            paths.extend(
                ["source", "target"]
                    .into_iter()
                    .filter_map(|key| relationship.get(key).and_then(Value::as_str))
                    .map(str::to_owned),
            );
        }
    }
    paths
}

fn working_tree_path_is_missing(
    repository: &std::path::Path,
    status: &git::WorkingTreeStatus,
    path: &str,
) -> bool {
    !repository.join(path).exists()
        || status.deleted_paths.iter().any(|deleted| deleted == path)
        || status
            .relationships
            .iter()
            .any(|relationship| relationship.source == path)
}

fn filter_status(
    status: &git::WorkingTreeStatus,
    paths: &BTreeSet<String>,
) -> git::WorkingTreeStatus {
    let contains = |path: &String| paths.contains(path);
    git::WorkingTreeStatus {
        committed_paths: status
            .committed_paths
            .iter()
            .filter(|path| contains(path))
            .cloned()
            .collect(),
        added_paths: status
            .added_paths
            .iter()
            .filter(|path| contains(path))
            .cloned()
            .collect(),
        modified_paths: status
            .modified_paths
            .iter()
            .filter(|path| contains(path))
            .cloned()
            .collect(),
        untracked_paths: status
            .untracked_paths
            .iter()
            .filter(|path| contains(path))
            .cloned()
            .collect(),
        staged_paths: status
            .staged_paths
            .iter()
            .filter(|path| contains(path))
            .cloned()
            .collect(),
        unstaged_paths: status
            .unstaged_paths
            .iter()
            .filter(|path| contains(path))
            .cloned()
            .collect(),
        deleted_paths: status
            .deleted_paths
            .iter()
            .filter(|path| contains(path))
            .cloned()
            .collect(),
        conflicted_paths: status
            .conflicted_paths
            .iter()
            .filter(|path| contains(path))
            .cloned()
            .collect(),
        relationships: status
            .relationships
            .iter()
            .filter(|relationship| {
                paths.contains(&relationship.source) || paths.contains(&relationship.target)
            })
            .cloned()
            .collect(),
    }
}
