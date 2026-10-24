//! Coocoo desktop app: a native window and tray around the Coocoo daemon.
//!
//! The daemon runs in-process (or, when `coocoo serve` already serves this config, the window
//! attaches to it). The UI is the same Svelte app the daemon serves to browsers and phones; it
//! learns where to connect from `window.__COOCOO__`, injected before the page loads.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;
mod watch;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use coocoo_core::Daemon;
use coocoo_proto::Paths;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::NewWindowResponse;
use tauri::{App, AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;

use crate::backend::Backend;

const MAIN_WINDOW: &str = "main";
/// Upper bound for a graceful daemon stop on quit; agents stay resumable either way.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

/// App-wide state shared by window, tray and run-loop handlers.
struct Shell {
    /// The in-process daemon (`None` when attached to an external one).
    daemon: Option<Arc<Daemon>>,
    /// Set once the daemon shutdown on quit has started.
    quitting: AtomicBool,
    /// Whether the main window has keyboard focus (notifications are skipped while it has).
    focused: Arc<AtomicBool>,
}

fn main() {
    init_tracing();
    let app = tauri::Builder::default()
        // Must be first: a second launch hands over to the running instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| show_main_window(app)))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // Visibility is not restored: quitting from the tray leaves the window hidden.
                .with_state_flags(StateFlags::all() - StateFlags::VISIBLE)
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        // Without the plugin's injected click handler: it would turn target=_blank clicks into
        // `plugin:opener|open_url` IPC calls, which the webview's capability does not grant, so
        // links did nothing. They reach `on_new_window` below instead.
        .plugin(tauri_plugin_opener::Builder::new().open_js_links_on_click(false).build())
        .setup(setup)
        .on_window_event(on_window_event)
        .build(tauri::generate_context!())
        .expect("failed to initialize the Coocoo desktop app");
    app.run(on_run_event);
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let backend = tauri::async_runtime::block_on(Backend::connect(Paths::from_env()))?;
    let focused = Arc::new(AtomicBool::new(false));
    app.manage(Shell { daemon: backend.daemon.clone(), quitting: AtomicBool::new(false), focused: focused.clone() });

    create_main_window(app.handle(), &backend)?;
    let status_item = create_tray(app.handle())?;
    watch::spawn(app.handle().clone(), &backend, status_item, focused);
    // Clicking a notification brings the app forward.
    let handle = app.handle().clone();
    app.notification().on_action(move |_| show_main_window(&handle))?;
    spawn_signal_handler(app.handle().clone());
    Ok(())
}

/// Built here rather than in `tauri.conf.json` so the bootstrap script carries runtime values.
fn create_main_window(app: &AppHandle, backend: &Backend) -> tauri::Result<()> {
    let bootstrap = serde_json::json!({ "wsUrl": backend.ws_url, "token": backend.token, "native": true });
    let window = WebviewWindowBuilder::new(app, MAIN_WINDOW, WebviewUrl::default())
        .title("Coocoo")
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 600.0)
        .center()
        .decorations(true)
        // Shown once the window-state plugin has applied the saved geometry.
        .visible(false)
        .initialization_script(format!("window.__COOCOO__ = {bootstrap};"))
        // The app has one window: links with target=_blank (chat links, URL actions, release
        // downloads) and window.open go to the system browser.
        .on_new_window({
            let app = app.clone();
            move |url, _features| {
                if matches!(url.scheme(), "http" | "https" | "mailto")
                    && let Err(err) = app.opener().open_url(url.as_str(), None::<&str>)
                {
                    tracing::warn!("cannot open {url}: {err}");
                }
                NewWindowResponse::Deny
            }
        })
        .build()?;
    // The UI is a single document fed over WebSocket: WebKit's page/resource caches only hold
    // memory. The document-viewer model disables them.
    #[cfg(target_os = "linux")]
    window.with_webview(|webview| {
        use webkit2gtk::{CacheModel, WebContextExt, WebViewExt};
        if let Some(context) = webview.inner().context() {
            context.set_cache_model(CacheModel::DocumentViewer);
        }
    })?;
    window.show()?;
    window.set_focus()
}

/// Tray icon with "Show", a live agent summary line and "Quit". Returns the summary item.
fn create_tray(app: &AppHandle) -> tauri::Result<MenuItem<tauri::Wry>> {
    let show = MenuItem::with_id(app, "show", "Show Coocoo", true, None::<&str>)?;
    let status = MenuItem::with_id(app, "status", watch::IDLE_LABEL, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Coocoo", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &status, &PredefinedMenuItem::separator(app)?, &quit])?;
    let mut tray = TrayIconBuilder::with_id(watch::TRAY_ID)
        .tooltip("Coocoo")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(status)
}

fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else { return };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW {
        return;
    }
    match event {
        // Closing hides to the tray; agents keep running. "Quit" in the tray exits.
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = window.hide();
            set_focused(window.app_handle(), false);
        }
        WindowEvent::Focused(focused) => set_focused(window.app_handle(), *focused),
        _ => {}
    }
}

fn set_focused(app: &AppHandle, focused: bool) {
    if let Some(shell) = app.try_state::<Shell>() {
        shell.focused.store(focused, Ordering::Relaxed);
    }
}

fn on_run_event(app: &AppHandle, event: RunEvent) {
    match event {
        // Every exit path (tray Quit, signals, OS logout) stops the embedded daemon first.
        RunEvent::ExitRequested { api, code, .. } => {
            let Some(shell) = app.try_state::<Shell>() else { return };
            let Some(daemon) = shell.daemon.clone() else { return };
            if shell.quitting.swap(true, Ordering::SeqCst) {
                return;
            }
            api.prevent_exit();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if tokio::time::timeout(SHUTDOWN_GRACE, daemon.shutdown()).await.is_err() {
                    tracing::warn!("daemon shutdown exceeded {SHUTDOWN_GRACE:?}; exiting anyway");
                }
                app.exit(code.unwrap_or(0));
            });
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_main_window(app),
        _ => {}
    }
}

/// SIGTERM/SIGINT/SIGHUP (Ctrl+C on Windows) quit gracefully, like the tray's Quit.
fn spawn_signal_handler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let (Ok(mut term), Ok(mut int), Ok(mut hup)) =
                (signal(SignalKind::terminate()), signal(SignalKind::interrupt()), signal(SignalKind::hangup()))
            else {
                tracing::warn!("cannot install signal handlers");
                return;
            };
            tokio::select! {
                _ = term.recv() => {}
                _ = int.recv() => {}
                _ = hup.recv() => {}
            }
        }
        #[cfg(not(unix))]
        if tokio::signal::ctrl_c().await.is_err() {
            return;
        }
        tracing::info!("signal received; quitting");
        app.exit(0);
    });
}
