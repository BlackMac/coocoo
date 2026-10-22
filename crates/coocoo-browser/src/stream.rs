//! Connection-scoped streams: screencast frame envelopes and console entries.

use std::sync::Arc;

use coocoo_proto::events;
use serde_json::Value;

/// Serialize a `browser.frame` envelope once for all watchers of a tab.
///
/// Built by hand so the (large) base64 payload is copied exactly once, straight from CDP into
/// the wire text. Base64 never needs JSON escaping.
pub(crate) fn frame_envelope(host: &str, profile: &str, target_id: &str, data: &str, width: u32, height: u32, seq: u64) -> Arc<str> {
    let quote = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned());
    let (host, profile, target_id) = (quote(host), quote(profile), quote(target_id));
    let mut out = String::with_capacity(data.len() + host.len() + profile.len() + target_id.len() + 128);
    out.push_str(r#"{"event":""#);
    out.push_str(events::BROWSER_FRAME);
    out.push_str(r#"","host":"#);
    out.push_str(&host);
    out.push_str(r#","data":{"profile":"#);
    out.push_str(&profile);
    out.push_str(r#","targetId":"#);
    out.push_str(&target_id);
    out.push_str(r#","data":""#);
    out.push_str(data);
    out.push_str(&format!(r#"","width":{width},"height":{height},"seq":{seq}}}}}"#));
    Arc::from(out)
}

/// Whether `s` is plain base64 (safe to embed unescaped).
pub(crate) fn is_base64(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
}

/// `(level, text)` for a console-relevant CDP event, `None` for anything else.
pub(crate) fn console_entry(method: &str, params: &Value) -> Option<(&'static str, String)> {
    match method {
        "Runtime.consoleAPICalled" => {
            let level = match params["type"].as_str().unwrap_or("log") {
                "warning" => "warning",
                "error" | "assert" => "error",
                "debug" => "debug",
                "info" => "info",
                _ => "log",
            };
            let text = params["args"]
                .as_array()
                .map(|args| args.iter().map(remote_object_text).collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            Some((level, text))
        }
        "Runtime.exceptionThrown" => {
            let d = &params["exceptionDetails"];
            let mut text = d["exception"]["description"]
                .as_str()
                .or_else(|| d["text"].as_str())
                .unwrap_or("Uncaught exception")
                .to_owned();
            if let Some(url) = d["url"].as_str().filter(|u| !u.is_empty()) {
                let line = d["lineNumber"].as_u64().unwrap_or(0) + 1;
                let col = d["columnNumber"].as_u64().unwrap_or(0) + 1;
                text.push_str(&format!("\n    at {url}:{line}:{col}"));
            }
            Some(("exception", text))
        }
        "Log.entryAdded" => {
            let e = &params["entry"];
            let level = if e["source"].as_str() == Some("network") {
                "network"
            } else {
                match e["level"].as_str().unwrap_or("info") {
                    "verbose" => "debug",
                    "warning" => "warning",
                    "error" => "error",
                    _ => "info",
                }
            };
            let mut text = e["text"].as_str().unwrap_or_default().to_owned();
            if let Some(url) = e["url"].as_str().filter(|u| !u.is_empty() && !text.contains(*u)) {
                text.push_str(" (");
                text.push_str(url);
                text.push(')');
            }
            Some((level, text))
        }
        _ => None,
    }
}

/// Human text for a `Runtime.RemoteObject` console argument.
fn remote_object_text(obj: &Value) -> String {
    match &obj["value"] {
        Value::String(s) => return s.clone(),
        Value::Null => {}
        v => return v.to_string(),
    }
    if obj["subtype"].as_str() == Some("null") {
        return "null".to_owned();
    }
    obj["unserializableValue"]
        .as_str()
        .or_else(|| obj["description"].as_str())
        .or_else(|| obj["type"].as_str())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use coocoo_proto::{BrowserFrame, EventEnvelope};
    use serde_json::json;

    use super::*;

    #[test]
    fn frame_envelope_round_trips() {
        let json = frame_envelope("h\"1", "p", "T", "/9j/4AA+=", 1280, 800, 7);
        let env: EventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(env.event, events::BROWSER_FRAME);
        assert_eq!(env.host, "h\"1");
        let frame: BrowserFrame = serde_json::from_value(env.data).unwrap();
        assert_eq!((frame.profile.as_str(), frame.target_id.as_str(), frame.data.as_str()), ("p", "T", "/9j/4AA+="));
        assert_eq!((frame.width, frame.height, frame.seq), (1280, 800, 7));
        assert!(is_base64("/9j/4AA+="));
        assert!(!is_base64("a\"b"));
    }

    #[test]
    fn console_mapping() {
        let (level, text) = console_entry(
            "Runtime.consoleAPICalled",
            &json!({"type":"warning","args":[{"type":"string","value":"clicked"},{"type":"number","value":3},
                {"type":"object","subtype":"null","value":null},{"type":"number","unserializableValue":"NaN"},
                {"type":"object","description":"Array(2)"}]}),
        )
        .unwrap();
        assert_eq!(level, "warning");
        assert_eq!(text, "clicked 3 null NaN Array(2)");

        let (level, text) = console_entry(
            "Runtime.exceptionThrown",
            &json!({"exceptionDetails":{"text":"Uncaught","exception":{"description":"Error: boom"},"url":"http://a/x.js","lineNumber":1,"columnNumber":4}}),
        )
        .unwrap();
        assert_eq!(level, "exception");
        assert_eq!(text, "Error: boom\n    at http://a/x.js:2:5");

        let (level, text) = console_entry(
            "Log.entryAdded",
            &json!({"entry":{"source":"network","level":"error","text":"Failed to load resource: 404","url":"http://a/missing.png"}}),
        )
        .unwrap();
        assert_eq!(level, "network");
        assert_eq!(text, "Failed to load resource: 404 (http://a/missing.png)");

        let (level, _) = console_entry("Log.entryAdded", &json!({"entry":{"source":"other","level":"verbose","text":"v"}})).unwrap();
        assert_eq!(level, "debug");
        assert!(console_entry("Page.loadEventFired", &json!({})).is_none());
    }
}
