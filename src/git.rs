//! Read-only Git metadata adapter.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use thiserror::Error;

/// Read-only metadata required to register a Git working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryMetadata {
    pub root: PathBuf,
    pub branch: Option<String>,
    pub head_commit: Option<String>,
}

/// A rename or copy relationship reported by Git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRelationship {
    pub kind: PathRelationshipKind,
    pub source: String,
    pub target: String,
}

/// The exact relationship Git reported for a changed path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRelationshipKind {
    Renamed,
    Copied,
}

/// A single read-only snapshot of a repository working tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkingTreeStatus {
    pub added_paths: Vec<String>,
    pub modified_paths: Vec<String>,
    pub untracked_paths: Vec<String>,
    pub staged_paths: Vec<String>,
    pub unstaged_paths: Vec<String>,
    pub deleted_paths: Vec<String>,
    pub relationships: Vec<PathRelationship>,
}

/// A safe Git command failure.
#[derive(Debug, Error)]
pub enum GitError {
    #[error("Git could not inspect {path}: {message}")]
    Command { path: PathBuf, message: String },
    #[error("Git reported repository root {path}, but PMEMC could not canonicalize it: {source}")]
    Canonicalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Git returned an invalid commit count for {path}: {value}")]
    InvalidCommitCount { path: PathBuf, value: String },
}

/// Read Git repository metadata without reading source files.
pub fn metadata(path: &Path) -> Result<RepositoryMetadata, GitError> {
    let root = run(path, &["rev-parse", "--show-toplevel"])?;
    let root = PathBuf::from(root.trim());
    let root =
        fs::canonicalize(&root).map_err(|source| GitError::Canonicalize { path: root, source })?;
    let branch = optional(&root, &["branch", "--show-current"])?;
    let head_commit = optional(&root, &["rev-parse", "HEAD"])?;
    Ok(RepositoryMetadata {
        root,
        branch,
        head_commit,
    })
}

/// Read all required working-tree changes in one Git status invocation.
///
/// This reads Git metadata only; it does not open repository source files.
pub fn working_tree_status(path: &Path) -> Result<WorkingTreeStatus, GitError> {
    let output = run_bytes(
        path,
        &[
            "-c",
            "status.renames=true",
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
        ],
    )?;
    let mut status = parse_working_tree_status(&output);
    status
        .relationships
        .extend(staged_exact_copy_relationships(path, &status.added_paths)?);
    Ok(status)
}

fn staged_exact_copy_relationships(
    path: &Path,
    added_paths: &[String],
) -> Result<Vec<PathRelationship>, GitError> {
    if added_paths.is_empty() || optional(path, &["rev-parse", "--verify", "HEAD"])?.is_none() {
        return Ok(Vec::new());
    }

    let head_paths_by_blob =
        paths_by_blob(&run_bytes(path, &["ls-tree", "-r", "-z", "HEAD"])?, true);
    let index_paths_by_blob =
        paths_by_blob(&run_bytes(path, &["ls-files", "--stage", "-z"])?, false);

    Ok(added_paths
        .iter()
        .filter_map(|target| {
            let blob = index_paths_by_blob
                .iter()
                .find_map(|(blob, paths)| paths.contains(target).then_some(blob))?;
            let source = head_paths_by_blob
                .get(blob)?
                .iter()
                .find(|path| *path != target)?;
            Some(PathRelationship {
                kind: PathRelationshipKind::Copied,
                source: source.clone(),
                target: target.clone(),
            })
        })
        .collect())
}

fn paths_by_blob(output: &[u8], from_head: bool) -> BTreeMap<String, Vec<String>> {
    output
        .split(|byte| *byte == 0)
        .filter_map(|record| {
            let separator = record.iter().position(|byte| *byte == b'\t')?;
            let metadata = &record[..separator];
            let path = &record[separator + 1..];
            let mut fields = metadata.split(|byte| *byte == b' ');
            let _mode = fields.next()?;
            let blob = if from_head {
                (fields.next()? == b"blob").then_some(fields.next()?)?
            } else {
                fields.next()?
            };
            Some((
                String::from_utf8_lossy(blob).into_owned(),
                String::from_utf8_lossy(path).into_owned(),
            ))
        })
        .fold(BTreeMap::new(), |mut paths_by_blob, (blob, path)| {
            paths_by_blob.entry(blob).or_default().push(path);
            paths_by_blob
        })
}

/// Count commits made since a previously observed commit without changing Git state.
pub fn commit_count_since(path: &Path, observed_commit: &str) -> Result<u64, GitError> {
    let revision_range = format!("{observed_commit}..HEAD");
    let count = run(path, &["rev-list", "--count", &revision_range])?;
    count
        .trim()
        .parse()
        .map_err(|_| GitError::InvalidCommitCount {
            path: path.into(),
            value: count,
        })
}

fn parse_working_tree_status(output: &[u8]) -> WorkingTreeStatus {
    let mut status = WorkingTreeStatus::default();
    let mut records = output.split(|byte| *byte == 0);

    while let Some(record) = records.next() {
        if let Some(path) = record.strip_prefix(b"? ") {
            status
                .untracked_paths
                .push(String::from_utf8_lossy(path).into_owned());
            continue;
        }

        let Some((xy, path)) = ordinary_record(record)
            .or_else(|| relationship_record(record, &mut records, &mut status))
        else {
            continue;
        };
        collect_path_states(&mut status, xy, path);
    }

    status
}

fn ordinary_record(record: &[u8]) -> Option<(&[u8], String)> {
    let mut fields = record.splitn(9, |byte| *byte == b' ');
    (fields.next()? == b"1").then_some(())?;
    let xy = fields.next()?;
    let path = fields.nth(6)?;
    Some((xy, String::from_utf8_lossy(path).into_owned()))
}

fn relationship_record<'a>(
    record: &'a [u8],
    records: &mut impl Iterator<Item = &'a [u8]>,
    status: &mut WorkingTreeStatus,
) -> Option<(&'a [u8], String)> {
    let mut fields = record.splitn(10, |byte| *byte == b' ');
    (fields.next()? == b"2").then_some(())?;
    let xy = fields.next()?;
    let kind = match fields.nth(6)?.first()? {
        b'R' => PathRelationshipKind::Renamed,
        b'C' => PathRelationshipKind::Copied,
        _ => return None,
    };
    let target = fields.next()?;
    let source = records.next()?;
    status.relationships.push(PathRelationship {
        kind,
        source: String::from_utf8_lossy(source).into_owned(),
        target: String::from_utf8_lossy(target).into_owned(),
    });
    Some((xy, String::from_utf8_lossy(target).into_owned()))
}

fn collect_path_states(status: &mut WorkingTreeStatus, xy: &[u8], path: String) {
    if xy.contains(&b'A') {
        status.added_paths.push(path.clone());
    }
    if xy.contains(&b'M') {
        status.modified_paths.push(path.clone());
    }
    if xy.first().is_some_and(|state| *state != b'.') {
        status.staged_paths.push(path.clone());
    }
    if xy.get(1).is_some_and(|state| *state != b'.') {
        status.unstaged_paths.push(path.clone());
    }
    if xy.contains(&b'D') {
        status.deleted_paths.push(path);
    }
}

fn optional(path: &Path, arguments: &[&str]) -> Result<Option<String>, GitError> {
    match run(path, arguments) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value.trim().into())),
        Err(_) => Ok(None),
    }
}
fn run(path: &Path, arguments: &[&str]) -> Result<String, GitError> {
    let output = run_bytes(path, arguments)?;
    Ok(String::from_utf8_lossy(&output).into_owned())
}

fn run_bytes(path: &Path, arguments: &[&str]) -> Result<Vec<u8>, GitError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(path)
        .output()
        .map_err(|error| GitError::Command {
            path: path.into(),
            message: error.to_string(),
        })?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(GitError::Command {
            path: path.into(),
            message: String::from_utf8_lossy(&output.stderr).trim().into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{PathRelationship, PathRelationshipKind, parse_working_tree_status};

    #[test]
    fn porcelain_copy_relationship_preserves_target_then_source_order() {
        let status = parse_working_tree_status(
            b"2 C. N... 100644 100644 100644 abcdef abcdef C100 copy.txt\0source.txt\0",
        );

        assert_eq!(status.staged_paths, ["copy.txt"]);
        assert_eq!(
            status.relationships,
            [PathRelationship {
                kind: PathRelationshipKind::Copied,
                source: "source.txt".into(),
                target: "copy.txt".into(),
            }]
        );
    }
}
