//! `.coocoo/persona.toml`: the assistant a project is the home of (name, avatar, color, voice, …).
//! Coocoo shows it and honors `keepAwake`; everything else is metadata for the assistant's own
//! tooling. Read from the project's main checkout like `actions.toml`: invalid keys are dropped
//! and reported with their line, a missing or invalid `name` means no persona.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use base64::Engine;
use coocoo_proto::{ApiError, ApiResult, Persona};
use serde_json::{Value, json};
use toml::Spanned;

use crate::actions::{DIR, first_line, position, read_config};

pub(crate) const FILE: &str = "persona.toml";
const MAX_NAME: usize = 80;
/// Larger files are not avatars (or not worth sending to every client).
const MAX_AVATAR: u64 = 1024 * 1024;
/// Avatar extensions and their MIME types.
const AVATAR_TYPES: &[(&str, &str)] = &[("png", "image/png"), ("jpg", "image/jpeg"), ("jpeg", "image/jpeg"), ("webp", "image/webp")];

#[derive(Debug, Default)]
pub(crate) struct Loaded {
    pub persona: Option<Persona>,
    pub errors: Vec<String>,
    /// `:LINE:COL` of the `avatar` key, for problems found when checking the file.
    avatar_at: String,
}

impl Loaded {
    fn error(message: String) -> Self {
        Self { errors: vec![message], ..Self::default() }
    }
}

/// Read and validate `<project>/.coocoo/persona.toml` and check its avatar file; no file = no
/// persona, no errors.
pub(crate) async fn load(project: &Path) -> Loaded {
    let mut loaded = match read_config(project, FILE).await {
        Ok(Some(src)) => parse(&src),
        Ok(None) => return Loaded::default(),
        Err(message) => return Loaded::error(message),
    };
    if let Some(persona) = loaded.persona.as_mut()
        && let Some(rel) = persona.avatar.clone()
    {
        match avatar_file(project, &rel).await {
            Ok(_) => persona.has_avatar = true,
            Err(message) => {
                persona.avatar = None;
                loaded.errors.push(format!("{FILE}{} – {message}", loaded.avatar_at));
            }
        }
    }
    loaded
}

/// `projects.personaAvatar`: the persona's avatar as `{ mime, data }` (base64).
pub(crate) async fn read_avatar(project: &Path, persona: Option<&Persona>) -> ApiResult<Value> {
    let rel = persona.and_then(|p| p.avatar.as_deref()).ok_or_else(|| ApiError::not_found("persona avatar"))?;
    let file = avatar_file(project, rel).await.map_err(ApiError::not_found)?;
    let mime = mime(rel).ok_or_else(|| ApiError::internal(format!("unexpected avatar file {rel}")))?;
    let bytes = tokio::fs::read(&file).await?;
    if bytes.len() as u64 > MAX_AVATAR {
        return Err(ApiError::conflict(format!("{rel} grew larger than {} KiB", MAX_AVATAR / 1024)));
    }
    Ok(json!({ "mime": mime, "data": base64::engine::general_purpose::STANDARD.encode(bytes) }))
}

/// The avatar file at `rel` (relative to the project) when it is a non-empty file of at most
/// `MAX_AVATAR` bytes inside `.coocoo/`, symlinks resolved.
async fn avatar_file(project: &Path, rel: &str) -> Result<PathBuf, String> {
    let dir = tokio::fs::canonicalize(project.join(DIR)).await.map_err(|_| format!("avatar {rel} not found"))?;
    let file = tokio::fs::canonicalize(project.join(rel)).await.map_err(|_| format!("avatar {rel} not found"))?;
    if !file.starts_with(&dir) {
        return Err(format!("avatar {rel} leads outside {DIR}/"));
    }
    let meta = tokio::fs::metadata(&file).await.map_err(|err| format!("avatar {rel}: {err}"))?;
    if !meta.is_file() || meta.len() == 0 {
        return Err(format!("avatar {rel} is not an image file"));
    }
    if meta.len() > MAX_AVATAR {
        return Err(format!("avatar {rel} is larger than {} KiB", MAX_AVATAR / 1024));
    }
    Ok(file)
}

fn mime(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    AVATAR_TYPES.iter().find(|(e, _)| *e == ext).map(|(_, mime)| *mime)
}

pub(crate) fn parse(src: &str) -> Loaded {
    let table: BTreeMap<Spanned<String>, Spanned<toml::Value>> = match toml::from_str(src) {
        Ok(table) => table,
        Err(err) => {
            let at = err.span().map_or_else(String::new, |span| position(src, span.start));
            return Loaded::error(format!("{FILE}{at} – {}", first_line(err.message())));
        }
    };
    let mut entries: Vec<(Spanned<String>, Spanned<toml::Value>)> = table.into_iter().collect();
    entries.sort_by_key(|(key, _)| key.span().start);

    let mut out = Loaded::default();
    let mut name = None;
    let mut persona = Persona {
        name: String::new(),
        color: None,
        voice: None,
        wake_word: None,
        keep_awake: false,
        has_avatar: false,
        avatar: None,
        extra: None,
    };
    let mut extra = serde_json::Map::new();
    for (key, value) in entries {
        let at = position(src, key.span().start);
        let key = key.into_inner();
        let value = value.into_inner();
        let result = match key.as_str() {
            "name" => string(&key, value).and_then(|n| valid_name(&n)).map(|n| name = Some(n)),
            "color" => string(&key, value).and_then(|c| valid_color(&c)).map(|c| persona.color = Some(c)),
            "voice" => string(&key, value).map(|v| persona.voice = Some(v)),
            "wakeWord" => string(&key, value).map(|w| persona.wake_word = Some(w.trim().to_owned()).filter(|w| !w.is_empty())),
            "keepAwake" => match value {
                toml::Value::Boolean(b) => {
                    persona.keep_awake = b;
                    Ok(())
                }
                _ => Err("`keepAwake` must be true or false".to_owned()),
            },
            "avatar" => {
                out.avatar_at = at.clone();
                string(&key, value).and_then(|a| valid_avatar(&a)).map(|a| persona.avatar = Some(a))
            }
            _ => {
                extra.insert(key, to_json(value));
                Ok(())
            }
        };
        if let Err(message) = result {
            out.errors.push(format!("{FILE}{at} – {message}"));
        }
    }
    let Some(name) = name else {
        if !out.errors.iter().any(|e| e.contains("`name`")) {
            out.errors.insert(0, format!("{FILE}:1:1 – `name` is required"));
        }
        return out;
    };
    persona.name = name;
    persona.extra = (!extra.is_empty()).then_some(extra);
    out.persona = Some(persona);
    out
}

fn string(key: &str, value: toml::Value) -> Result<String, String> {
    match value {
        toml::Value::String(s) => Ok(s),
        _ => Err(format!("`{key}` must be a string")),
    }
}

fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("`name` is empty".to_owned());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("`name` is longer than {MAX_NAME} characters"));
    }
    Ok(name.to_owned())
}

fn valid_color(color: &str) -> Result<String, String> {
    let color = color.trim();
    let hex = color.strip_prefix('#').filter(|h| h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()));
    hex.map(|_| color.to_owned()).ok_or_else(|| "`color` must look like #RRGGBB".to_owned())
}

/// `avatar` relative to `.coocoo/` → the path relative to the project directory.
fn valid_avatar(avatar: &str) -> Result<String, String> {
    let mut parts = vec![DIR.to_owned()];
    for component in Path::new(avatar.trim()).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return Err(format!("`avatar` must be a path inside {DIR}/ (no `..`)")),
        }
    }
    let path = parts.join("/");
    if parts.len() < 2 || mime(&path).is_none() {
        return Err("`avatar` must be a .png, .jpg or .webp file".to_owned());
    }
    Ok(path)
}

fn to_json(value: toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::from(i),
        toml::Value::Float(f) => serde_json::Number::from_f64(f).map_or_else(|| Value::String(f.to_string()), Value::Number),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(items) => Value::Array(items.into_iter().map(to_json).collect()),
        toml::Value::Table(table) => Value::Object(table.into_iter().map(|(k, v)| (k, to_json(v))).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_file_with_defaults_and_extra_keys() {
        let loaded = parse(
            r##"
name = " Orlando "
avatar = "./avatars/orlando.PNG"
color = "#3A7bd5"
voice = " de-DE-Florian "
wakeWord = "Hey Orlando"
language = "de"
tools = ["calendar", "mail"]

[telegram]
chat = 42
since = 2026-01-02
"##,
        );
        assert_eq!(loaded.errors, Vec::<String>::new());
        let p = loaded.persona.unwrap();
        assert_eq!(p.name, "Orlando");
        assert_eq!(p.avatar.as_deref(), Some(".coocoo/avatars/orlando.PNG"));
        // The file is checked by `load`, not here.
        assert!(!p.has_avatar);
        assert_eq!(p.color.as_deref(), Some("#3A7bd5"));
        assert_eq!(p.voice.as_deref(), Some(" de-DE-Florian "));
        assert_eq!(p.wake_word.as_deref(), Some("Hey Orlando"));
        assert!(!p.keep_awake);
        assert_eq!(
            Value::Object(p.extra.unwrap()),
            json!({ "language": "de", "tools": ["calendar", "mail"], "telegram": { "chat": 42, "since": "2026-01-02" } })
        );
        let wire = serde_json::to_value(parse("name = \"A\"\nkeepAwake = true\n").persona.unwrap()).unwrap();
        assert_eq!(wire, json!({ "name": "A", "keepAwake": true, "hasAvatar": false }));
    }

    #[test]
    fn invalid_keys_are_dropped_with_their_line() {
        let loaded = parse(
            "name = \"Angelica\"\ncolor = \"red\"\nkeepAwake = \"yes\"\nvoice = 3\navatar = \"../secret.png\"\nwakeWord = \"Angelica\"\n",
        );
        let p = loaded.persona.unwrap();
        assert_eq!((p.color, p.voice, p.avatar, p.keep_awake), (None, None, None, false));
        assert_eq!(p.wake_word.as_deref(), Some("Angelica"));
        assert_eq!(
            loaded.errors,
            [
                "persona.toml:2:1 – `color` must look like #RRGGBB",
                "persona.toml:3:1 – `keepAwake` must be true or false",
                "persona.toml:4:1 – `voice` must be a string",
                "persona.toml:5:1 – `avatar` must be a path inside .coocoo/ (no `..`)",
            ]
        );
        for (avatar, message) in [
            ("/etc/a.png", "`avatar` must be a path inside .coocoo/ (no `..`)"),
            ("face.gif", "`avatar` must be a .png, .jpg or .webp file"),
            (".", "`avatar` must be a .png, .jpg or .webp file"),
        ] {
            let loaded = parse(&format!("name = \"n\"\navatar = \"{avatar}\"\n"));
            assert_eq!(loaded.errors, [format!("persona.toml:2:1 – {message}")], "{avatar}");
        }
    }

    #[test]
    fn without_a_valid_name_there_is_no_persona() {
        let loaded = parse("color = \"#000000\"\n");
        assert!(loaded.persona.is_none());
        assert_eq!(loaded.errors, ["persona.toml:1:1 – `name` is required"]);
        let loaded = parse("color = \"#000000\"\nname = \"  \"\n");
        assert!(loaded.persona.is_none());
        assert_eq!(loaded.errors, ["persona.toml:2:1 – `name` is empty"]);
        let loaded = parse("name = [\"a\"]\n");
        assert_eq!(loaded.errors, ["persona.toml:1:1 – `name` must be a string"]);
        let loaded = parse("name = \"a\"\nvoice = \"b\n");
        assert!(loaded.persona.is_none());
        assert_eq!(loaded.errors.len(), 1);
        assert!(loaded.errors[0].starts_with("persona.toml:2:"), "{:?}", loaded.errors);
    }

    #[tokio::test]
    async fn load_checks_the_avatar_file() {
        let dir = std::env::temp_dir().join(format!("coocoo-persona-{}", coocoo_proto::util::new_id()));
        assert!(load(&dir).await.persona.is_none() && load(&dir).await.errors.is_empty());
        std::fs::create_dir_all(dir.join(DIR).join("img")).unwrap();
        let write = |toml: &str| std::fs::write(dir.join(DIR).join(FILE), toml).unwrap();

        write("name = \"Orlando\"\navatar = \"img/o.webp\"\n");
        let loaded = load(&dir).await;
        assert!(!loaded.persona.as_ref().unwrap().has_avatar);
        assert_eq!(loaded.errors, ["persona.toml:2:1 – avatar .coocoo/img/o.webp not found"]);

        std::fs::write(dir.join(DIR).join("img/o.webp"), b"RIFF").unwrap();
        let loaded = load(&dir).await;
        assert_eq!(loaded.errors, Vec::<String>::new());
        let persona = loaded.persona.unwrap();
        assert!(persona.has_avatar);
        let avatar = read_avatar(&dir, Some(&persona)).await.unwrap();
        assert_eq!(avatar, json!({ "mime": "image/webp", "data": "UklGRg==" }));
        assert_eq!(read_avatar(&dir, None).await.unwrap_err().code, "not_found");

        #[cfg(unix)]
        {
            std::fs::write(dir.join("outside.png"), b"x").unwrap();
            std::os::unix::fs::symlink(dir.join("outside.png"), dir.join(DIR).join("link.png")).unwrap();
            write("name = \"Orlando\"\navatar = \"link.png\"\n");
            let loaded = load(&dir).await;
            assert_eq!(loaded.errors, ["persona.toml:2:1 – avatar .coocoo/link.png leads outside .coocoo/"]);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
