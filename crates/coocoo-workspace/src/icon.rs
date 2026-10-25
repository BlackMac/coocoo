//! Project icons: the favicon, app icon or logo a project keeps at one of the usual places
//! (web `public/`/`static/`, Next.js `app/`, Tauri `src-tauri/icons/`, Flutter `web/`, Android
//! `mipmap-*`, …), in the project root or one directory down (monorepos: `cockpit/public/icons/`).

use std::path::Path;
use std::time::UNIX_EPOCH;

use base64::Engine;
use coocoo_proto::{ApiError, ApiResult, ProjectIcon};
use serde_json::{Value, json};

/// Directories searched below the root and below each top-level directory, most conventional first.
const DIRS: &[&str] = &[
    "",
    "public",
    "public/icons",
    "static",
    "static/icons",
    "app",
    "src/app",
    "assets",
    "assets/icons",
    "src/assets",
    "src/assets/icons",
    "src",
    "web",
    "web/icons",
    "src-tauri/icons",
    "src/main/res/mipmap-xxxhdpi",
    "app/src/main/res/mipmap-xxxhdpi",
    "resources",
    "build",
    ".github",
    "docs",
];
/// Top-level directories not searched as nested projects (hidden ones are skipped too).
const SKIP: &[&str] = &["node_modules", "target", "vendor", "dist", "build", "Pods", "DerivedData"];
/// File stems, best first: square icons beat logos, which are often wide wordmarks.
const NAMES: &[&str] = &[
    "favicon",
    "icon",
    "app-icon",
    "apple-touch-icon",
    "apple-icon",
    "icon-192",
    "logo192",
    "128x128",
    "ic_launcher",
    "logo",
];
/// Extensions, best first (vector, then raster).
const EXTS: &[&str] = &["svg", "png", "webp", "ico"];
/// Larger files are not icons (or not worth sending to every client).
const MAX_BYTES: u64 = 512 * 1024;

/// Best icon file in `dir`, if any. Reads one directory listing per searched place.
pub(crate) async fn find(dir: &Path) -> Option<ProjectIcon> {
    let dir = dir.to_owned();
    tokio::task::spawn_blocking(move || find_blocking(&dir)).await.ok().flatten()
}

/// (name rank, ext rank, depth, dir rank, top-level directory rank): lowest wins.
type Rank = (usize, usize, usize, usize, usize);
type Best = Option<(Rank, String, i64)>;

fn find_blocking(root: &Path) -> Option<ProjectIcon> {
    let mut best: Best = None;
    let mut children = search(root, "", (0, 0), &mut best);
    children.retain(|n| !n.starts_with('.') && !SKIP.contains(&n.as_str()));
    children.sort_unstable();
    for (i, child) in children.iter().enumerate() {
        search(root, child, (1, i), &mut best);
    }
    best.map(|(_, path, modified)| ProjectIcon { path, modified })
}

/// Considers the icon files at the `DIRS` places below `root/base` (only those whose first
/// directory exists); returns the directories (and symlinks) directly in `root/base`.
fn search(root: &Path, base: &str, (depth, base_rank): (usize, usize), best: &mut Best) -> Vec<String> {
    let mut subdirs = Vec::new();
    for (dir_rank, sub) in DIRS.iter().enumerate() {
        // `""` comes first and fills `subdirs`.
        if let Some(top) = sub.split('/').next().filter(|t| !t.is_empty())
            && !subdirs.iter().any(|d| d == top)
        {
            continue;
        }
        let rel = match (base, *sub) {
            ("", s) | (s, "") => s.to_owned(),
            (b, s) => format!("{b}/{s}"),
        };
        let Ok(entries) = std::fs::read_dir(root.join(&rel)) else { continue };
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else { continue };
            if sub.is_empty() && entry.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink()) {
                subdirs.push(file_name.to_owned());
            }
            let lower = file_name.to_ascii_lowercase();
            let Some((stem, ext)) = lower.rsplit_once('.') else { continue };
            let (Some(name_rank), Some(ext_rank)) =
                (NAMES.iter().position(|n| *n == stem), EXTS.iter().position(|e| *e == ext))
            else {
                continue;
            };
            let rank = (name_rank, ext_rank, depth, dir_rank, base_rank);
            if best.as_ref().is_some_and(|(r, ..)| *r <= rank) {
                continue;
            }
            // Follows symlinks; skips directories, empty and oversized files.
            let Ok(meta) = std::fs::metadata(entry.path()) else { continue };
            if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_BYTES {
                continue;
            }
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as i64);
            let path = if rel.is_empty() { file_name.to_owned() } else { format!("{rel}/{file_name}") };
            *best = Some((rank, path, modified));
        }
    }
    subdirs
}

/// `projects.icon`: the detected file as `{ mime, data }` (base64).
pub(crate) async fn read(root: &Path, icon: &ProjectIcon) -> ApiResult<Value> {
    let mime = match icon.path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref() {
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        _ => return Err(ApiError::internal(format!("unexpected icon file {}", icon.path))),
    };
    let bytes = tokio::fs::read(root.join(&icon.path)).await?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(ApiError::conflict(format!("{} grew larger than {MAX_BYTES} bytes", icon.path)));
    }
    Ok(json!({ "mime": mime, "data": base64::engine::general_purpose::STANDARD.encode(bytes) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[&str]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("coocoo-icon-{}", coocoo_proto::util::new_id()));
        for f in files {
            let path = root.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        }
        root
    }

    fn found(files: &[&str]) -> Option<String> {
        let root = project(files);
        let icon = find_blocking(&root).map(|i| i.path);
        std::fs::remove_dir_all(root).ok();
        icon
    }

    #[test]
    fn picks_icons_over_logos_and_vector_over_raster() {
        assert_eq!(found(&["docs/logo.svg", "public/favicon.ico"]).as_deref(), Some("public/favicon.ico"));
        assert_eq!(found(&["public/favicon.ico", "static/favicon.svg"]).as_deref(), Some("static/favicon.svg"));
        assert_eq!(found(&["src-tauri/icons/icon.svg", "src-tauri/icons/128x128.png"]).as_deref(), Some("src-tauri/icons/icon.svg"));
        assert_eq!(found(&["README.md", "logo.png"]).as_deref(), Some("logo.png"));
    }

    #[test]
    fn root_wins_ties_and_case_is_ignored() {
        assert_eq!(found(&["public/favicon.png", "Favicon.PNG"]).as_deref(), Some("Favicon.PNG"));
    }

    #[test]
    fn searches_one_level_down_after_the_root() {
        assert_eq!(found(&["cockpit/public/icons/icon.svg", "README.md"]).as_deref(), Some("cockpit/public/icons/icon.svg"));
        assert_eq!(
            found(&["android/app/src/main/res/mipmap-xxxhdpi/ic_launcher.png", "ios/Icon1024.png"]).as_deref(),
            Some("android/app/src/main/res/mipmap-xxxhdpi/ic_launcher.png")
        );
        assert_eq!(found(&["web/public/favicon.svg", "favicon.svg"]).as_deref(), Some("favicon.svg"));
        assert_eq!(found(&["b/public/favicon.png", "a/public/favicon.png"]).as_deref(), Some("a/public/favicon.png"));
        assert_eq!(found(&["node_modules/favicon.svg", ".hidden/favicon.svg", "a/x/favicon.svg"]), None);
    }

    #[test]
    fn nothing_and_non_files() {
        assert_eq!(found(&["src/main.rs"]), None);
        let root = project(&["src/main.rs"]);
        std::fs::create_dir_all(root.join("icon.png")).unwrap();
        std::fs::write(root.join("favicon.ico"), b"").unwrap();
        assert_eq!(find_blocking(&root), None);
        std::fs::remove_dir_all(root).ok();
    }
}
