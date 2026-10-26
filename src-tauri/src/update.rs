//! Auto-update checks, downloads, installations, and restart coordination.
//!
//! Checks periodically against GitHub Releases `latest.json` (signed with the minisign key),
//! surfaces updates via tray icon, native notifications, and frontend IPC commands.
//! Before restart, in-progress agents pause via `daemon.drain()` and the daemon shuts down cleanly.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::Shell;

pub const UPDATE_STATUS_EVENT: &str = "update://status";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UpdateStatus {
    /// Idle before any check has completed.
    Idle {
        current_version: String,
    },
    /// In an environment where updates cannot be applied.
    #[allow(dead_code)]
    Unsupported {
        current_version: String,
        reason: String,
    },
    /// Currently checking for updates.
    Checking {
        current_version: String,
    },
    /// The current version is up to date.
    UpToDate {
        current_version: String,
        checked_at: u64,
    },
    /// A newer version is available to download and install.
    Available {
        version: String,
        current_version: String,
        body: Option<String>,
        date: Option<String>,
    },
    /// Downloading and preparing the update.
    Downloading {
        version: String,
        downloaded: usize,
        total: Option<u64>,
        percent: Option<u8>,
    },
    /// The update is installed on disk; restart to run it.
    ReadyToRestart {
        version: String,
    },
    /// An error occurred during check or installation.
    Error {
        message: String,
        current_version: String,
    },
}

pub struct UpdateManager {
    app: AppHandle,
    status: RwLock<UpdateStatus>,
    cached_update: tokio::sync::Mutex<Option<Update>>,
    installing: AtomicBool,
    last_notified_version: Mutex<Option<String>>,
    tray: Mutex<Option<(Menu<tauri::Wry>, MenuItem<tauri::Wry>)>>,
}

impl UpdateManager {
    pub fn new(app: AppHandle) -> Self {
        let current_version = app.package_info().version.to_string();
        Self {
            app,
            status: RwLock::new(UpdateStatus::Idle { current_version }),
            cached_update: tokio::sync::Mutex::new(None),
            installing: AtomicBool::new(false),
            last_notified_version: Mutex::new(None),
            tray: Mutex::new(None),
        }
    }

    pub fn set_tray(&self, menu: Menu<tauri::Wry>, item: MenuItem<tauri::Wry>) {
        let _ = menu.remove(&item);
        *self.tray.lock().unwrap() = Some((menu, item));
    }

    pub fn current_status(&self) -> UpdateStatus {
        self.status.read().unwrap().clone()
    }

    fn set_status(&self, new_status: UpdateStatus) {
        *self.status.write().unwrap() = new_status.clone();
        let _ = self.app.emit(UPDATE_STATUS_EVENT, &new_status);
        self.sync_tray(&new_status);
    }

    fn sync_tray(&self, status: &UpdateStatus) {
        let Some((menu, item)) = self.tray.lock().unwrap().clone() else {
            return;
        };
        match status {
            UpdateStatus::Available { version, .. } => {
                let _ = item.set_text(format!("Update to v{version} (restart)"));
                let _ = item.set_enabled(true);
                if menu.get(item.id()).is_none() {
                    let _ = menu.insert(&item, 2);
                }
            }
            UpdateStatus::ReadyToRestart { version } => {
                let _ = item.set_text(format!("Restart to update to v{version}"));
                let _ = item.set_enabled(true);
                if menu.get(item.id()).is_none() {
                    let _ = menu.insert(&item, 2);
                }
            }
            UpdateStatus::Downloading { percent, version, .. } => {
                let pct = percent.map(|p| format!(" ({p}%)")).unwrap_or_default();
                let _ = item.set_text(format!("Downloading v{version}{pct}…"));
                let _ = item.set_enabled(false);
                if menu.get(item.id()).is_none() {
                    let _ = menu.insert(&item, 2);
                }
            }
            _ => {
                let _ = menu.remove(&item);
            }
        }
    }

    fn notify_available(&self, version: &str) {
        let mut last = self.last_notified_version.lock().unwrap();
        if last.as_deref() == Some(version) {
            return;
        }
        *last = Some(version.to_string());
        let _ = self.app.notification().builder()
            .title("Coocoo update available")
            .body(format!("Version {version} is ready to install."))
            .show();
    }

    pub async fn check(&self) -> Result<UpdateStatus, String> {
        let current_version = self.app.package_info().version.to_string();

        // Check if updater is supported in this build/packaging.
        #[cfg(all(not(target_os = "macos"), not(windows), not(debug_assertions)))]
        if tauri_utils::platform::bundle_type().is_none() {
            let st = UpdateStatus::Unsupported {
                current_version,
                reason: "Auto-update is available for AppImage, deb, macOS, and Windows installs.".into(),
            };
            self.set_status(st.clone());
            return Ok(st);
        }

        self.set_status(UpdateStatus::Checking {
            current_version: current_version.clone(),
        });

        let updater = match self.app.updater() {
            Ok(u) => u,
            Err(err) => {
                let st = UpdateStatus::Error {
                    message: format!("Updater not configured: {err}"),
                    current_version,
                };
                self.set_status(st.clone());
                return Err(err.to_string());
            }
        };

        match updater.check().await {
            Ok(Some(update)) => {
                let st = UpdateStatus::Available {
                    version: update.version.clone(),
                    current_version: update.current_version.clone(),
                    body: update.body.clone(),
                    date: update.date.map(|d| d.to_string()),
                };
                let version = update.version.clone();
                *self.cached_update.lock().await = Some(update);
                self.set_status(st.clone());
                self.notify_available(&version);
                Ok(st)
            }
            Ok(None) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let st = UpdateStatus::UpToDate {
                    current_version,
                    checked_at: now,
                };
                *self.cached_update.lock().await = None;
                self.set_status(st.clone());
                Ok(st)
            }
            Err(err) => {
                let st = UpdateStatus::Error {
                    message: err.to_string(),
                    current_version,
                };
                self.set_status(st.clone());
                Err(err.to_string())
            }
        }
    }

    pub async fn install_and_restart(self: &Arc<Self>) -> Result<(), String> {
        if self.installing.swap(true, Ordering::SeqCst) {
            return Err("Installation already in progress".into());
        }

        let update = {
            let guard = self.cached_update.lock().await;
            if guard.is_none() {
                drop(guard);
                if let Err(err) = self.check().await {
                    self.installing.store(false, Ordering::SeqCst);
                    return Err(err);
                }
                self.cached_update.lock().await.clone()
            } else {
                guard.clone()
            }
        };

        let Some(update) = update else {
            self.installing.store(false, Ordering::SeqCst);
            return Err("No update available".into());
        };

        let version = update.version.clone();
        self.set_status(UpdateStatus::Downloading {
            version: version.clone(),
            downloaded: 0,
            total: None,
            percent: None,
        });

        let downloaded_acc = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let manager_clone = self.clone();
        let ver = version.clone();

        let bytes = update
            .download(
                {
                    let downloaded_acc = downloaded_acc.clone();
                    let manager = manager_clone.clone();
                    let ver = ver.clone();
                    move |chunk_len, total| {
                        let cur = downloaded_acc.fetch_add(chunk_len, Ordering::Relaxed) + chunk_len;
                        let percent = total.map(|t| ((cur as f64 / t as f64) * 100.0).min(100.0) as u8);
                        manager.set_status(UpdateStatus::Downloading {
                            version: ver.clone(),
                            downloaded: cur,
                            total,
                            percent,
                        });
                    }
                },
                || {},
            )
            .await
            .map_err(|e| {
                let msg = e.to_string();
                manager_clone.set_status(UpdateStatus::Error {
                    message: msg.clone(),
                    current_version: manager_clone.app.package_info().version.to_string(),
                });
                manager_clone.installing.store(false, Ordering::SeqCst);
                msg
            })?;

        #[cfg(not(windows))]
        {
            // Install the payload into place before stopping the daemon.
            // If verification or installation fails (e.g. pkexec cancelled),
            // the running app and daemon remain untouched.
            let update_clone = update.clone();
            let bytes_clone = bytes.clone();
            let install_res = tokio::task::spawn_blocking(move || update_clone.install(&bytes_clone))
                .await
                .map_err(|e| e.to_string())?;

            if let Err(err) = install_res {
                let msg = err.to_string();
                tracing::error!("failed to install update: {msg}");
                self.set_status(UpdateStatus::Error {
                    message: msg.clone(),
                    current_version: self.app.package_info().version.to_string(),
                });
                self.installing.store(false, Ordering::SeqCst);
                return Err(msg);
            }

            self.set_status(UpdateStatus::ReadyToRestart { version: version.clone() });

            // Graceful shutdown: pause agents and stop in-process daemon.
            self.graceful_shutdown().await;

            tracing::info!("restarting to launch updated version {version}");
            self.app.restart();
        }

        #[cfg(windows)]
        {
            // On Windows, `update.install` runs the NSIS/MSI installer and exits the process (`exit(0)`).
            // Therefore we must gracefully drain and stop the daemon first.
            self.graceful_shutdown().await;

            let update_clone = update.clone();
            let install_res = tokio::task::spawn_blocking(move || update_clone.install(&bytes))
                .await
                .map_err(|e| e.to_string())?;

            if let Err(err) = install_res {
                let msg = err.to_string();
                tracing::error!("failed to launch Windows installer: {msg}");
                self.installing.store(false, Ordering::SeqCst);
                // Try restarting the current version to recover the daemon
                self.app.restart();
                return Err(msg);
            }
            Ok(())
        }
    }

    async fn graceful_shutdown(&self) {
        if let Some(shell) = self.app.try_state::<Shell>() {
            if let Some(daemon) = &shell.daemon {
                tracing::info!("asking running agents to pause before update restart");
                let _ = tokio::time::timeout(Duration::from_secs(30), daemon.drain()).await;
                tracing::info!("stopping daemon before update restart");
                let _ = tokio::time::timeout(Duration::from_secs(10), daemon.shutdown()).await;
            }
            shell.quitting.store(true, Ordering::SeqCst);
        }
    }
}

pub fn spawn_background_checker(manager: Arc<UpdateManager>) {
    tauri::async_runtime::spawn(async move {
        // Initial delay so startup is fast and quiet.
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            tracing::info!("running periodic desktop update check");
            let _ = manager.check().await;
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}

// ─── Tauri IPC Commands ────────────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn update_status(
    manager: tauri::State<'_, Arc<UpdateManager>>,
) -> Result<UpdateStatus, String> {
    Ok(manager.current_status())
}

#[tauri::command]
pub async fn update_check(
    manager: tauri::State<'_, Arc<UpdateManager>>,
) -> Result<UpdateStatus, String> {
    manager.check().await
}

#[tauri::command]
pub async fn update_install(
    manager: tauri::State<'_, Arc<UpdateManager>>,
) -> Result<(), String> {
    manager.install_and_restart().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_status_serialization() {
        let status = UpdateStatus::Available {
            version: "0.6.0".into(),
            current_version: "0.5.0".into(),
            body: Some("Release notes".into()),
            date: Some("2026-10-26T12:00:00Z".into()),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains(r#""status":"available""#));
        assert!(json.contains(r#""version":"0.6.0""#));
        assert!(json.contains(r#""currentVersion":"0.5.0""#));
        assert!(json.contains(r#""body":"Release notes""#));

        let downloading = UpdateStatus::Downloading {
            version: "0.6.0".into(),
            downloaded: 1024,
            total: Some(2048),
            percent: Some(50),
        };
        let json = serde_json::to_string(&downloading).unwrap();
        assert!(json.contains(r#""status":"downloading""#));
        assert!(json.contains(r#""downloaded":1024"#));
        assert!(json.contains(r#""percent":50"#));

        let up_to_date = UpdateStatus::UpToDate {
            current_version: "0.5.0".into(),
            checked_at: 1700000000,
        };
        let json = serde_json::to_string(&up_to_date).unwrap();
        assert!(json.contains(r#""status":"upToDate""#));
        assert!(json.contains(r#""checkedAt":1700000000"#));
    }
}
