use std::path::PathBuf;

/// Storage locations.
///
/// Defaults follow the platform conventions (`~/.config/coocoo`, `~/.local/share/coocoo`,
/// `~/.cache/coocoo` on Linux). Setting `COOCOO_HOME` puts everything under that directory,
/// which allows several independent daemons on one machine (tests, multi-host simulation).
#[derive(Debug, Clone)]
pub struct Paths {
    /// `config.toml` (host id, token, listen address, peers).
    pub config_dir: PathBuf,
    /// Durable state: `projects.json`, `agents/<id>/…`, browser profiles.
    pub data_dir: PathBuf,
    /// Disposable: logs, worktrees default root, temp files.
    pub cache_dir: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        if let Some(home) = std::env::var_os("COOCOO_HOME").filter(|v| !v.is_empty()) {
            return Self::under(PathBuf::from(home));
        }
        let base = |d: Option<PathBuf>, fallback: &str| {
            d.unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(fallback))
                .join("coocoo")
        };
        Self {
            config_dir: base(dirs::config_dir(), ".config"),
            data_dir: base(dirs::data_dir(), ".local/share"),
            cache_dir: base(dirs::cache_dir(), ".cache"),
        }
    }

    pub fn under(root: PathBuf) -> Self {
        Self { config_dir: root.join("config"), data_dir: root.join("data"), cache_dir: root.join("cache") }
    }

    /// Address the running daemon is bound to (`serve --listen` may differ from config.toml);
    /// written on start, removed on shutdown.
    pub fn listen_file(&self) -> PathBuf {
        self.cache_dir.join("listen")
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [&self.config_dir, &self.data_dir, &self.cache_dir] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}
