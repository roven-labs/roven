//! Safe, deterministic repository file inventory.

use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use serde::Serialize;
use thiserror::Error;

/// One file eligible for structural extraction or generic fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InventoryFile {
    pub path: String,
    pub language: Language,
}

/// The supported structural language or safe fallback classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Language {
    Rust,
    Python,
    Java,
    Go,
    JavaScript,
    TypeScript,
    Jsx,
    Tsx,
    GenericText,
    Unsupported,
}

/// A stable inventory of files eligible for inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    pub files: Vec<InventoryFile>,
}

/// Errors that prevent Git-aware inventory construction.
#[derive(Debug, Error)]
pub enum InventoryError {
    #[error("PMEMC could not inventory {path}: {message}")]
    Git { path: PathBuf, message: String },
    #[error("PMEMC could not read ignore rules from {path}: {source}")]
    IgnoreRules {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Inventory inspectable files without executing repository code.
///
/// Git supplies the `.gitignore`-aware path set. PMEMC reads at most a small
/// prefix of each candidate to reject binary files; it does not execute code.
pub fn inventory(repository: &Path) -> Result<Inventory, InventoryError> {
    let root = fs::canonicalize(repository).map_err(|source| InventoryError::Git {
        path: repository.into(),
        message: source.to_string(),
    })?;
    let patterns = pmemc_ignore_patterns(&root)?;
    let output = git_paths(&root)?;
    let mut files = output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .filter(|path| {
            !is_default_excluded(path) && !patterns.iter().any(|pattern| matches(pattern, path))
        })
        .filter(|path| !is_binary(&root.join(path)))
        .map(|path| InventoryFile {
            language: language_for(&path),
            path,
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(Inventory { files })
}

fn git_paths(root: &Path) -> Result<Vec<u8>, InventoryError> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .output()
        .map_err(|source| InventoryError::Git {
            path: root.into(),
            message: source.to_string(),
        })?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(InventoryError::Git {
            path: root.into(),
            message: String::from_utf8_lossy(&output.stderr).trim().into(),
        })
    }
}

fn pmemc_ignore_patterns(root: &Path) -> Result<Vec<String>, InventoryError> {
    let path = root.join(".pmemcignore");
    match fs::read_to_string(&path) {
        Ok(contents) => Ok(contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_owned)
            .collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(InventoryError::IgnoreRules { path, source }),
    }
}

fn is_default_excluded(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    filename == ".env"
        || filename.starts_with(".env.")
        || [".pem", ".key", ".p12", ".pfx"]
            .iter()
            .any(|extension| filename.ends_with(extension))
        || path.split('/').any(|component| {
            matches!(
                component,
                ".git" | "node_modules" | "vendor" | "target" | "build" | "dist" | "out"
            )
        })
}

fn is_binary(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return true;
    };
    let mut prefix = [0_u8; 8192];
    match file.read(&mut prefix) {
        Ok(read) => prefix[..read].contains(&0),
        Err(_) => true,
    }
}

fn matches(pattern: &str, path: &str) -> bool {
    if !pattern.contains('*') {
        return pattern.trim_end_matches('/').eq(path);
    }
    let mut remaining = path;
    let anchored = !pattern.starts_with('*');
    for (index, segment) in pattern
        .split('*')
        .filter(|segment| !segment.is_empty())
        .enumerate()
    {
        let Some(position) = remaining.find(segment) else {
            return false;
        };
        if index == 0 && anchored && position != 0 {
            return false;
        }
        remaining = &remaining[position + segment.len()..];
    }
    pattern.ends_with('*') || remaining.is_empty()
}

fn language_for(path: &str) -> Language {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("rs") => Language::Rust,
        Some("py") => Language::Python,
        Some("java") => Language::Java,
        Some("go") => Language::Go,
        Some("js" | "mjs" | "cjs") => Language::JavaScript,
        Some("ts" | "mts" | "cts") => Language::TypeScript,
        Some("jsx") => Language::Jsx,
        Some("tsx") => Language::Tsx,
        Some("md" | "markdown" | "json" | "yaml" | "yml" | "toml" | "ipynb") => {
            Language::GenericText
        }
        _ => Language::Unsupported,
    }
}
