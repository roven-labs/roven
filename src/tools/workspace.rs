use std::{
    io,
    path::{Component, Path, PathBuf},
};

use super::ToolContext;

pub(super) enum WorkspacePathError {
    InvalidPath,
    PathNotAllowed,
    PermissionDenied,
    IoError,
}

pub(super) fn canonical_workspace_path(
    context: &ToolContext,
    path: &str,
) -> Result<PathBuf, WorkspacePathError> {
    let relative = Path::new(path);
    if relative
        .components()
        .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        return Err(WorkspacePathError::InvalidPath);
    }
    if relative
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(WorkspacePathError::PathNotAllowed);
    }
    if relative.components().any(|component| matches!(component, Component::Normal(name) if name.to_string_lossy().contains(':'))) {
        return Err(WorkspacePathError::InvalidPath);
    }
    let target = context
        .trusted_workspace
        .join(relative)
        .canonicalize()
        .map_err(io_error_reason)?;
    if !target.starts_with(&context.trusted_workspace) {
        return Err(WorkspacePathError::PathNotAllowed);
    }
    Ok(target)
}

pub(super) fn workspace_relative_path(root: &Path, path: &Path) -> String {
    let relative = path
        .strip_prefix(root)
        .expect("authorized path remains under trusted workspace");
    workspace_relative_path_from_relative(relative)
}

pub(super) fn workspace_relative_path_from_relative(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    if path.is_empty() {
        ".".to_owned()
    } else {
        path
    }
}

fn io_error_reason(error: io::Error) -> WorkspacePathError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => WorkspacePathError::PermissionDenied,
        io::ErrorKind::NotFound => WorkspacePathError::InvalidPath,
        _ => WorkspacePathError::IoError,
    }
}
