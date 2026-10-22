//! `browser.input` events → CDP `Input.*` commands, and the navigation URL policy.

use coocoo_proto::{ApiError, ApiResult};
use serde::Deserialize;
use serde_json::{Value, json};

/// CDP modifier bits.
const ALT: u32 = 1;
const CTRL: u32 = 2;
const META: u32 = 4;
const SHIFT: u32 = 8;

/// Modifiers as sent by clients: CDP bitmask (`Alt=1, Ctrl=2, Meta=4, Shift=8`), a list of
/// names (`["ctrl","shift"]`), or flags (`{ctrl:true, shift:true}`).
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
pub(crate) enum Modifiers {
    #[default]
    None,
    Mask(u32),
    Names(Vec<String>),
    Flags {
        #[serde(default, alias = "altKey")]
        alt: bool,
        #[serde(default, alias = "ctrlKey", alias = "control")]
        ctrl: bool,
        #[serde(default, alias = "metaKey", alias = "cmd", alias = "command")]
        meta: bool,
        #[serde(default, alias = "shiftKey")]
        shift: bool,
    },
}

impl Modifiers {
    pub fn mask(&self) -> u32 {
        match self {
            Self::None => 0,
            Self::Mask(m) => m & (ALT | CTRL | META | SHIFT),
            Self::Names(names) => names
                .iter()
                .map(|n| match n.to_ascii_lowercase().as_str() {
                    "alt" | "option" => ALT,
                    "ctrl" | "control" => CTRL,
                    "meta" | "cmd" | "command" | "super" => META,
                    "shift" => SHIFT,
                    _ => 0,
                })
                .fold(0, |a, b| a | b),
            Self::Flags { alt, ctrl, meta, shift } => {
                [(alt, ALT), (ctrl, CTRL), (meta, META), (shift, SHIFT)]
                    .iter()
                    .filter(|(on, _)| **on)
                    .fold(0, |acc, (_, bit)| acc | bit)
            }
        }
    }
}

/// Mouse button by DOM name or `MouseEvent.button` number.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Button {
    Name(String),
    Index(u8),
}

impl Button {
    fn cdp(&self) -> ApiResult<&'static str> {
        let name = match self {
            Self::Name(n) => match n.as_str() {
                "none" => "none",
                "left" => "left",
                "middle" => "middle",
                "right" => "right",
                "back" => "back",
                "forward" => "forward",
                other => return Err(ApiError::invalid(format!("unknown mouse button `{other}`"))),
            },
            Self::Index(0) => "left",
            Self::Index(1) => "middle",
            Self::Index(2) => "right",
            Self::Index(3) => "back",
            Self::Index(4) => "forward",
            Self::Index(n) => return Err(ApiError::invalid(format!("unknown mouse button {n}"))),
        };
        Ok(name)
    }
}

/// `buttons` bitmask (CDP/DOM) for a pressed button.
fn buttons_mask(button: &str) -> u32 {
    match button {
        "left" => 1,
        "right" => 2,
        "middle" => 4,
        "back" => 8,
        "forward" => 16,
        _ => 0,
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum InputEvent {
    #[serde(rename_all = "camelCase")]
    Mouse {
        action: String,
        x: f64,
        y: f64,
        button: Option<Button>,
        click_count: Option<u32>,
        #[serde(default)]
        delta_x: f64,
        #[serde(default)]
        delta_y: f64,
        #[serde(default)]
        modifiers: Modifiers,
    },
    #[serde(rename_all = "camelCase")]
    Key {
        action: String,
        key: String,
        #[serde(default)]
        code: String,
        text: Option<String>,
        key_code: Option<u32>,
        #[serde(default)]
        modifiers: Modifiers,
    },
    Text { text: String },
}

/// CDP `(method, params)` for one input event.
pub(crate) fn to_cdp(event: InputEvent) -> ApiResult<(&'static str, Value)> {
    match event {
        InputEvent::Mouse { action, x, y, button, click_count, delta_x, delta_y, modifiers } => {
            if !x.is_finite() || !y.is_finite() {
                return Err(ApiError::invalid("mouse coordinates must be finite"));
            }
            let modifiers = modifiers.mask();
            let (kind, default_button, default_clicks) = match action.as_str() {
                "down" => ("mousePressed", "left", 1),
                "up" => ("mouseReleased", "left", 1),
                "move" => ("mouseMoved", "none", 0),
                "wheel" => ("mouseWheel", "none", 0),
                other => return Err(ApiError::invalid(format!("unknown mouse action `{other}`"))),
            };
            let button = match &button {
                Some(b) => b.cdp()?,
                None => default_button,
            };
            let mut params = json!({
                "type": kind,
                "x": x,
                "y": y,
                "modifiers": modifiers,
                "button": button,
                "clickCount": click_count.unwrap_or(default_clicks),
            });
            // While pressed (or dragging), report the held button so pages see `buttons`.
            if kind != "mouseReleased" {
                params["buttons"] = json!(buttons_mask(button));
            }
            if kind == "mouseWheel" {
                params["deltaX"] = json!(delta_x);
                params["deltaY"] = json!(delta_y);
            }
            Ok(("Input.dispatchMouseEvent", params))
        }
        InputEvent::Key { action, key, code, text, key_code, modifiers } => {
            let modifiers = modifiers.mask();
            let down = match action.as_str() {
                "down" => true,
                "up" => false,
                other => return Err(ApiError::invalid(format!("unknown key action `{other}`"))),
            };
            let vk = key_code.unwrap_or_else(|| virtual_key_code(&key, &code));
            let mut params = json!({
                "modifiers": modifiers,
                "key": key,
                "code": code,
                "windowsVirtualKeyCode": vk,
                "nativeVirtualKeyCode": vk,
            });
            if down {
                let text = if modifiers & (CTRL | META) != 0 { None } else { text.or_else(|| key_text(&key)) };
                match text {
                    Some(text) => {
                        params["type"] = json!("keyDown");
                        params["unmodifiedText"] = json!(text);
                        params["text"] = json!(text);
                    }
                    None => params["type"] = json!("rawKeyDown"),
                }
            } else {
                params["type"] = json!("keyUp");
            }
            Ok(("Input.dispatchKeyEvent", params))
        }
        InputEvent::Text { text } => Ok(("Input.insertText", json!({ "text": text }))),
    }
}

/// Text a key produces when the client did not send one: printable single characters, Enter.
fn key_text(key: &str) -> Option<String> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_control() => Some(key.to_owned()),
        _ if key == "Enter" => Some("\r".to_owned()),
        _ => None,
    }
}

/// Windows virtual key code from DOM `key` / `code`.
pub(crate) fn virtual_key_code(key: &str, code: &str) -> u32 {
    let named = match key {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" => 13,
        "Shift" => 16,
        "Control" => 17,
        "Alt" => 18,
        "Pause" => 19,
        "CapsLock" => 20,
        "Escape" => 27,
        " " => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Insert" => 45,
        "Delete" => 46,
        "Meta" => 91,
        "ContextMenu" => 93,
        _ => 0,
    };
    if named != 0 {
        return named;
    }
    if let Some(n) = key.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=24).contains(n)) {
        return 111 + n;
    }
    // Physical key position wins for letters/digits so shifted/layout characters map correctly.
    if let Some(c) = code.strip_prefix("Key").filter(|c| c.len() == 1) {
        return c.as_bytes()[0].to_ascii_uppercase() as u32;
    }
    if let Some(d) = code.strip_prefix("Digit").filter(|d| d.len() == 1) {
        return d.as_bytes()[0] as u32;
    }
    let punct = match code {
        "Semicolon" => 186,
        "Equal" => 187,
        "Comma" => 188,
        "Minus" => 189,
        "Period" => 190,
        "Slash" => 191,
        "Backquote" => 192,
        "BracketLeft" => 219,
        "Backslash" => 220,
        "BracketRight" => 221,
        "Quote" => 222,
        "Space" => 32,
        _ => 0,
    };
    if punct != 0 {
        return punct;
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase() as u32,
        _ => 0,
    }
}

/// Navigation policy for `browser.open` / `browser.navigate`: only http(s) and `about:blank`.
/// Scheme-less input (`localhost:5173`, `example.com/a`) gets `http://` for loopback hosts and
/// `https://` otherwise, like a URL bar.
pub(crate) fn normalize_url(input: &str) -> ApiResult<String> {
    let url = input.trim();
    if url.is_empty() {
        return Err(ApiError::invalid("url is empty"));
    }
    if url.eq_ignore_ascii_case("about:blank") {
        return Ok("about:blank".to_owned());
    }
    if let Some(colon) = url.find(':') {
        let scheme = &url[..colon];
        let rest = &url[colon + 1..];
        let looks_like_scheme = !scheme.is_empty()
            && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            // `host:port` is not a scheme.
            && !rest.chars().next().is_some_and(|c| c.is_ascii_digit());
        if looks_like_scheme {
            let lower = scheme.to_ascii_lowercase();
            return match lower.as_str() {
                "http" | "https" if rest.starts_with("//") && rest.len() > 2 => Ok(url.to_owned()),
                "http" | "https" => Err(ApiError::invalid(format!("malformed url `{url}`"))),
                _ => Err(ApiError::new("forbidden", format!("navigation to `{lower}:` URLs is not allowed"))),
            };
        }
    }
    if url.contains(char::is_whitespace) {
        return Err(ApiError::invalid(format!("not a url: `{url}`")));
    }
    let host = url.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).map_or(host, |(h, _)| h);
    let loopback = host == "localhost"
        || host.ends_with(".localhost")
        || host.starts_with("127.")
        || host == "0.0.0.0"
        || host == "[::1]";
    Ok(format!("{}://{url}", if loopback { "http" } else { "https" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: Value) -> InputEvent {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn modifiers_mapping() {
        let m = |v: Value| serde_json::from_value::<Modifiers>(v).unwrap().mask();
        assert_eq!(m(json!(null)), 0);
        assert_eq!(m(json!(10)), CTRL | SHIFT);
        assert_eq!(m(json!(255)), 15);
        assert_eq!(m(json!(["ctrl", "Shift", "bogus"])), CTRL | SHIFT);
        assert_eq!(m(json!(["alt", "cmd"])), ALT | META);
        assert_eq!(m(json!({"alt": true, "meta": true})), ALT | META);
        assert_eq!(m(json!({"ctrlKey": true, "shiftKey": false})), CTRL);
        assert_eq!(m(json!({})), 0);
    }

    #[test]
    fn mouse_events() {
        let (method, p) =
            to_cdp(parse(json!({"type":"mouse","action":"down","x":10,"y":20,"modifiers":["shift"]}))).unwrap();
        assert_eq!(method, "Input.dispatchMouseEvent");
        assert_eq!(p["type"], "mousePressed");
        assert_eq!(p["button"], "left");
        assert_eq!(p["buttons"], 1);
        assert_eq!(p["clickCount"], 1);
        assert_eq!(p["modifiers"], SHIFT);

        let (_, p) = to_cdp(parse(json!({"type":"mouse","action":"up","x":1,"y":2,"button":2}))).unwrap();
        assert_eq!(p["type"], "mouseReleased");
        assert_eq!(p["button"], "right");
        assert!(p.get("buttons").is_none());

        let (_, p) = to_cdp(parse(json!({"type":"mouse","action":"move","x":1,"y":2}))).unwrap();
        assert_eq!((p["type"].as_str(), p["button"].as_str(), p["buttons"].as_u64()), (Some("mouseMoved"), Some("none"), Some(0)));

        let (_, p) =
            to_cdp(parse(json!({"type":"mouse","action":"wheel","x":1,"y":2,"deltaY":120,"deltaX":-3}))).unwrap();
        assert_eq!(p["type"], "mouseWheel");
        assert_eq!(p["deltaY"], 120.0);
        assert_eq!(p["deltaX"], -3.0);

        assert!(to_cdp(parse(json!({"type":"mouse","action":"poke","x":1,"y":2}))).is_err());
        assert!(to_cdp(parse(json!({"type":"mouse","action":"down","x":1,"y":2,"button":"thumb"}))).is_err());
    }

    #[test]
    fn key_events() {
        let (method, p) = to_cdp(parse(json!({"type":"key","action":"down","key":"a","code":"KeyA"}))).unwrap();
        assert_eq!(method, "Input.dispatchKeyEvent");
        assert_eq!(p["type"], "keyDown");
        assert_eq!(p["text"], "a");
        assert_eq!(p["windowsVirtualKeyCode"], 65);

        let (_, p) = to_cdp(parse(json!({"type":"key","action":"down","key":"Enter","code":"Enter"}))).unwrap();
        assert_eq!((p["type"].as_str(), p["text"].as_str(), p["windowsVirtualKeyCode"].as_u64()), (Some("keyDown"), Some("\r"), Some(13)));

        let (_, p) = to_cdp(parse(json!({"type":"key","action":"down","key":"ArrowLeft","code":"ArrowLeft"}))).unwrap();
        assert_eq!(p["type"], "rawKeyDown");
        assert!(p.get("text").is_none());
        assert_eq!(p["windowsVirtualKeyCode"], 37);

        // Shortcuts carry no text so the page sees a command, not typing.
        let (_, p) =
            to_cdp(parse(json!({"type":"key","action":"down","key":"a","code":"KeyA","modifiers":{"ctrl":true}})))
                .unwrap();
        assert_eq!(p["type"], "rawKeyDown");
        assert_eq!(p["modifiers"], CTRL);

        let (_, p) = to_cdp(parse(json!({"type":"key","action":"up","key":"!","code":"Digit1","text":"!"}))).unwrap();
        assert_eq!(p["type"], "keyUp");
        assert_eq!(p["windowsVirtualKeyCode"], 49);

        let (_, p) =
            to_cdp(parse(json!({"type":"key","action":"down","key":"x","code":"KeyX","keyCode":999}))).unwrap();
        assert_eq!(p["windowsVirtualKeyCode"], 999);
        assert_eq!(virtual_key_code("F5", "F5"), 116);
        assert_eq!(virtual_key_code(".", "Period"), 190);
    }

    #[test]
    fn text_event() {
        let (method, p) = to_cdp(parse(json!({"type":"text","text":"héllo"}))).unwrap();
        assert_eq!(method, "Input.insertText");
        assert_eq!(p["text"], "héllo");
    }

    #[test]
    fn url_policy() {
        assert_eq!(normalize_url("https://example.com/a").unwrap(), "https://example.com/a");
        assert_eq!(normalize_url(" http://127.0.0.1:8080 ").unwrap(), "http://127.0.0.1:8080");
        assert_eq!(normalize_url("about:blank").unwrap(), "about:blank");
        assert_eq!(normalize_url("localhost:5173/x").unwrap(), "http://localhost:5173/x");
        assert_eq!(normalize_url("127.0.0.1:3000").unwrap(), "http://127.0.0.1:3000");
        assert_eq!(normalize_url("example.com").unwrap(), "https://example.com");
        for bad in ["file:///etc/passwd", "chrome://settings", "javascript:alert(1)", "about:config", "data:text/html,x", "view-source:https://a"] {
            assert_eq!(normalize_url(bad).unwrap_err().code, "forbidden", "{bad}");
        }
        assert!(normalize_url("").is_err());
        assert!(normalize_url("http:").is_err());
        assert!(normalize_url("two words").is_err());
    }
}
