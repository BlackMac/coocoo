//! `fs.list`: directory browsing for the "add project" picker.

use std::path::{Path, PathBuf};

use coocoo_proto::{ApiError, ApiResult, FsEntry};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Listing {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub entries: Vec<FsEntry>,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Expand a leading `~`; relative paths are taken relative to the home directory.
pub(crate) fn expand(path: &str) -> PathBuf {
    let path = path.trim();
    let expanded = if path.is_empty() || path == "~" {
        home()
    } else if let Some(rest) = path.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(path)
    };
    if expanded.is_absolute() { expanded } else { home().join(expanded) }
}

/// List a directory: directories first, then files, each case-insensitively by name;
/// hidden (dot) entries omitted.
pub(crate) async fn list(path: Option<&str>) -> ApiResult<Listing> {
    let dir = expand(path.unwrap_or(""));
    tokio::task::spawn_blocking(move || list_blocking(&dir)).await.map_err(ApiError::internal)?
}

fn list_blocking(dir: &Path) -> ApiResult<Listing> {
    let dir = std::fs::canonicalize(dir).map_err(|_| ApiError::not_found(format!("directory {}", dir.display())))?;
    if !dir.is_dir() {
        return Err(ApiError::invalid(format!("{} is not a directory", dir.display())));
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        // Follow symlinks so linked directories are browsable.
        let is_dir = std::fs::metadata(&path).is_ok_and(|m| m.is_dir());
        let is_git = is_dir && path.join(".git").exists();
        entries.push(FsEntry { name, path: path.to_string_lossy().into_owned(), is_dir, is_git });
    }
    entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase(), e.name.clone()));
    Ok(Listing {
        parent: dir.parent().map(|p| p.to_string_lossy().into_owned()),
        path: dir.to_string_lossy().into_owned(),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tilde() {
        let home = home();
        assert_eq!(expand("~"), home);
        assert_eq!(expand("~/x/y"), home.join("x/y"));
        assert_eq!(expand("/abs"), PathBuf::from("/abs"));
        assert_eq!(expand("rel"), home.join("rel"));
    }
}
