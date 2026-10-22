//! Project icons: the favicon, app icon or logo a project keeps at one of the usual places
//! (web `public/`/`static/`, Next.js `app/`, Tauri `src-tauri/icons/`, Flutter `web/`, …).

use std::path::Path;
use std::time::UNIX_EPOCH;

use base64::Engine;
use coocoo_proto::{ApiError, ApiResult, ProjectIcon};
use serde_json::{Value, json};

/// Directories searched, most conventional first.
const DIRS: &[&str] = &[
    "",
    "public",
    "static",
    "app",
    "src/app",
    "assets",
    "src/assets",
    "src",
    "web",
    "web/icons",
    "src-tauri/icons",
    "resources",
    "build",
    ".github",
    "docs",
];
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
    "logo",
];
/// Extensions, best first (vector, then raster).
const EXTS: &[&str] = &["svg", "png", "ico"];
/// Larger files are not icons (or not worth sending to every client).
const MAX_BYTES: u64 = 512 * 1024;

/// Best icon file in `dir`, if any. Reads one directory listing per searched place.
pub(crate) async fn find(dir: &Path) -> Option<ProjectIcon> {
    let dir = dir.to_owned();
    tokio::task::spawn_blocking(move || find_blocking(&dir)).await.ok().flatten()
}

fn find_blocking(root: &Path) -> Option<ProjectIcon> {
    // (name rank, ext rank, dir rank) — lowest wins.
    let mut best: Option<((usize, usize, usize), String, i64)> = None;
    for (dir_rank, sub) in DIRS.iter().enumerate() {
        let Ok(entries) = std::fs::read_dir(root.join(sub)) else { continue };
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else { continue };
            let lower = file_name.to_ascii_lowercase();
            let Some((stem, ext)) = lower.rsplit_once('.') else { continue };
            let (Some(name_rank), Some(ext_rank)) =
                (NAMES.iter().position(|n| *n == stem), EXTS.iter().position(|e| *e == ext))
            else {
                continue;
            };
            let rank = (name_rank, ext_rank, dir_rank);
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
            let path = if sub.is_empty() { file_name.to_owned() } else { format!("{sub}/{file_name}") };
            best = Some((rank, path, modified));
        }
    }
    best.map(|(_, path, modified)| ProjectIcon { path, modified })
}

/// `projects.icon`: the detected file as `{ mime, data }` (base64).
pub(crate) async fn read(root: &Path, icon: &ProjectIcon) -> ApiResult<Value> {
    let mime = match icon.path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref() {
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
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
    fn nothing_and_non_files() {
        assert_eq!(found(&["src/main.rs"]), None);
        let root = project(&["src/main.rs"]);
        std::fs::create_dir_all(root.join("icon.png")).unwrap();
        std::fs::write(root.join("favicon.ico"), b"").unwrap();
        assert_eq!(find_blocking(&root), None);
        std::fs::remove_dir_all(root).ok();
    }
}
