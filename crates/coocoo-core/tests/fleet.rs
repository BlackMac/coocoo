//! Two in-process daemons on ephemeral ports: auth, health, CDP routing, peer linking,
//! request forwarding, event relay, offline/reconnect.

use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use coocoo_core::{Daemon, DaemonOptions};
use coocoo_proto::{Paths, util};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{self, Message};

const STEP: Duration = Duration::from_secs(20);

struct TempHome(PathBuf);

impl TempHome {
    fn new(tag: &str) -> TempHome {
        // Never launch a real Chromium from these tests; CDP routing is asserted through the error.
        Self::with_config(tag, "[browser]\nchromePath = \"/nonexistent/coocoo-test-chrome\"\n")
    }

    fn with_config(tag: &str, config_toml: &str) -> TempHome {
        let dir = std::env::temp_dir().join(format!("coocoo-test-{tag}-{}", util::new_id()));
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("config.toml"), config_toml).unwrap();
        TempHome(dir)
    }

    fn paths(&self) -> Paths {
        Paths::under(self.0.clone())
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn start(home: &TempHome, listen: &str) -> Arc<Daemon> {
    Daemon::start(DaemonOptions { paths: home.paths(), listen: Some(listen.parse().unwrap()) }).await.unwrap()
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Client {
    ws: Ws,
    next: u64,
    events: Vec<Value>,
}

impl Client {
    async fn connect(daemon: &Daemon) -> Client {
        let url = format!("{}?token={}", daemon.ws_url(), daemon.token);
        let (ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        Client { ws, next: 1, events: Vec::new() }
    }

    async fn call(&mut self, host: Option<&str>, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.next;
        self.next += 1;
        let mut req = json!({ "id": id, "method": method, "params": params });
        if let Some(host) = host {
            req["host"] = json!(host);
        }
        self.ws.send(Message::Text(req.to_string().into())).await.unwrap();
        let wait = async {
            loop {
                let frame = self.next_frame().await;
                if frame.get("event").is_some() {
                    self.events.push(frame);
                } else if frame["id"] == json!(id) {
                    return match frame.get("error") {
                        Some(err) => Err(err.clone()),
                        None => Ok(frame["result"].clone()),
                    };
                }
            }
        };
        tokio::time::timeout(STEP, wait).await.unwrap_or_else(|_| panic!("{method} timed out"))
    }

    async fn next_frame(&mut self) -> Value {
        loop {
            match self.ws.next().await.expect("socket closed").unwrap() {
                Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
                _ => continue,
            }
        }
    }

    /// First buffered or future event matching `pred`.
    async fn event(&mut self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        if let Some(pos) = self.events.iter().position(&pred) {
            return self.events.remove(pos);
        }
        let mut skipped = Vec::new();
        let wait = async {
            loop {
                let frame = self.next_frame().await;
                if frame.get("event").is_some() && pred(&frame) {
                    return frame;
                }
                skipped.push(format!("{}@{}", frame["event"].as_str().unwrap_or("response"), frame["host"]));
            }
        };
        match tokio::time::timeout(STEP, wait).await {
            Ok(frame) => frame,
            Err(_) => panic!("no {what} event; saw {skipped:?}"),
        }
    }
}

fn http_get(addr: SocketAddr, path: &str, host: &str) -> (u16, String) {
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(STEP)).unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, body.to_owned())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_surface() {
    let home = TempHome::new("http");
    let daemon = start(&home, "127.0.0.1:0").await;
    let addr = daemon.local_addr;
    let host = addr.to_string();

    let (status, body) = tokio::task::spawn_blocking(move || http_get(addr, "/api/health", &host)).await.unwrap();
    assert_eq!(status, 200);
    let health: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(health["ok"], json!(true));
    assert_eq!(health["hostId"], json!(daemon.host_id));

    // Bad token: the upgrade is refused with 401.
    let bad = format!("{}?token=nope", daemon.ws_url());
    match tokio_tungstenite::connect_async(bad).await {
        Err(tungstenite::Error::Http(resp)) => assert_eq!(resp.status(), 401),
        other => panic!("expected 401, got {:?}", other.map(|_| ())),
    }

    // Static UI or the "not built" page; SPA routes fall back to it.
    let host = addr.to_string();
    let (status, _) = tokio::task::spawn_blocking(move || http_get(addr, "/agents/xyz", &host)).await.unwrap();
    assert_eq!(status, 200);

    let mut client = Client::connect(&daemon).await;
    let err = client.call(None, "nope.method", json!({})).await.unwrap_err();
    assert_eq!(err["code"], json!("unknown_method"));
    let err = client.call(Some("not-a-host"), "projects.list", json!({})).await.unwrap_err();
    assert_eq!(err["code"], json!("not_found"));

    // Each profile gets its own loopback discovery root (puppeteer drops any path from
    // `browser.cdpUrl`). Chromium cannot start here, so discovery answers with a JSON error;
    // a foreign Host header is refused.
    let res = client.call(None, "browser.cdpEndpoint", json!({ "profile": "default" })).await.unwrap();
    let url = res["url"].as_str().unwrap().to_owned();
    let again = client.call(None, "browser.cdpEndpoint", json!({ "profile": "default" })).await.unwrap();
    assert_eq!(again["url"].as_str(), Some(url.as_str()));
    let cdp: SocketAddr = url.strip_prefix("http://").unwrap().parse().unwrap();
    assert!(cdp.ip().is_loopback() && cdp.port() != addr.port(), "{url}");
    let host = cdp.to_string();
    let (status, body) = tokio::task::spawn_blocking(move || http_get(cdp, "/json/version", &host)).await.unwrap();
    assert_ne!(status, 200, "{body}");
    assert_ne!(status, 403, "{body}");
    let parsed: Value = serde_json::from_str(&body).unwrap_or_else(|_| panic!("not JSON: {body}"));
    assert!(parsed["code"].is_string(), "{body}");
    let (status, _) =
        tokio::task::spawn_blocking(move || http_get(cdp, "/json/version", "evil.example:80")).await.unwrap();
    assert_eq!(status, 403);
    let profiles = client.call(None, "browser.profiles", json!({})).await.unwrap();
    let default = profiles.as_array().unwrap().iter().find(|p| p["id"] == json!("default")).unwrap();
    assert_eq!(default["cdpUrl"].as_str(), Some(url.as_str()));
    let err = client.call(None, "browser.cdpEndpoint", json!({ "profile": "../x" })).await.unwrap_err();
    assert_eq!(err["code"], json!("invalid_params"));

    daemon.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_daemon_fleet() {
    let home_a = TempHome::new("a");
    let home_b = TempHome::new("b");
    let a = start(&home_a, "127.0.0.1:0").await;
    let b = start(&home_b, "127.0.0.1:0").await;
    let b_id = b.host_id.clone();
    let mut client = Client::connect(&a).await;

    let hello = client.call(None, "system.hello", Value::Null).await.unwrap();
    assert_eq!(hello["host"]["id"], json!(a.host_id));
    assert_eq!(hello["hosts"].as_array().unwrap().len(), 1);

    // Link B to A.
    let info = client
        .call(None, "hosts.add", json!({ "kind": "direct", "url": b.ws_url(), "token": b.token, "name": "box-b" }))
        .await
        .unwrap();
    assert_eq!(info["id"], json!(b_id));
    assert_eq!(info["online"], json!(true));
    assert_eq!(info["kind"], json!("direct"));
    assert_eq!(info["name"], json!("box-b"));
    let saved = std::fs::read_to_string(home_a.0.join("config/config.toml")).unwrap();
    assert!(saved.contains(&b_id), "peer id not persisted:\n{saved}");

    let hello = client.call(None, "system.hello", Value::Null).await.unwrap();
    let hosts = hello["hosts"].as_array().unwrap();
    let peer = hosts.iter().find(|h| h["id"] == json!(b_id)).expect("B in hello");
    assert_eq!(peer["online"], json!(true));

    // Adding the same daemon twice is rejected.
    let dup = client.call(None, "hosts.add", json!({ "kind": "direct", "url": b.ws_url(), "token": b.token })).await;
    assert_eq!(dup.unwrap_err()["code"], json!("conflict"));
    // Linking a daemon to itself is rejected.
    let me = client.call(None, "hosts.add", json!({ "kind": "direct", "url": a.ws_url(), "token": a.token })).await;
    assert!(me.is_err());

    // Forwarded request; B's event is relayed to A's client tagged with B.
    let project_dir = home_b.0.join("project");
    std::fs::create_dir_all(&project_dir).unwrap();
    let project =
        client.call(Some(&b_id), "projects.add", json!({ "path": project_dir.to_string_lossy() })).await.unwrap();
    assert_eq!(project["hostId"], json!(b_id));
    let event = client.event("project.updated", |e| e["event"] == json!("project.updated") && e["host"] == json!(b_id)).await;
    assert_eq!(event["data"]["id"], project["id"]);
    let listed = client.call(Some(&b_id), "projects.list", json!({})).await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let local = client.call(None, "projects.list", json!({})).await.unwrap();
    assert_eq!(local.as_array().unwrap().len(), 0);

    // Stream subscription through the link: B's stats reach A's client.
    client.call(Some(&b_id), "stats.watch", json!({ "enabled": true })).await.unwrap();
    client.event("host.stats", |e| e["event"] == json!("host.stats") && e["host"] == json!(b_id)).await;
    client.call(Some(&b_id), "stats.watch", json!({ "enabled": false })).await.unwrap();

    // Rename the peer locally.
    let renamed = client.call(None, "hosts.rename", json!({ "id": b_id, "name": "build-box" })).await.unwrap();
    assert_eq!(renamed["name"], json!("build-box"));

    // B goes away: A reports it offline and fails requests fast.
    let b_addr = b.local_addr;
    client.events.clear();
    b.shutdown().await;
    drop(b);
    client
        .event("host offline", |e| {
            e["event"] == json!("host.updated") && e["data"]["id"] == json!(b_id) && e["data"]["online"] == json!(false)
        })
        .await;
    let err = client.call(Some(&b_id), "projects.list", json!({})).await.unwrap_err();
    assert_eq!(err["code"], json!("unavailable"));

    // B comes back on the same address: the link reconnects on its own.
    client.events.clear();
    let b = start(&home_b, &b_addr.to_string()).await;
    assert_eq!(b.host_id, b_id);
    client
        .event("host online", |e| {
            e["event"] == json!("host.updated") && e["data"]["id"] == json!(b_id) && e["data"]["online"] == json!(true)
        })
        .await;
    let listed = client.call(Some(&b_id), "projects.list", json!({})).await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);

    // Remove B from the fleet.
    client.call(None, "hosts.remove", json!({ "id": b_id })).await.unwrap();
    client.event("host.removed", |e| e["event"] == json!("host.removed") && e["data"]["id"] == json!(b_id)).await;
    let hosts = client.call(None, "hosts.list", Value::Null).await.unwrap();
    assert_eq!(hosts.as_array().unwrap().len(), 1);
    let err = client.call(Some(&b_id), "projects.list", json!({})).await.unwrap_err();
    assert_eq!(err["code"], json!("not_found"));

    a.shutdown().await;
    b.shutdown().await;
}

/// A tab running on peer B is live-viewed by two clients of A through one remote stream; the
/// stream survives one viewer leaving. Needs a local Chromium (skipped otherwise).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_browser_stream() {
    let chrome = std::env::var("COOCOO_CHROME").ok().filter(|p| !p.is_empty()).or_else(|| {
        ["/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(str::to_owned)
    });
    let Some(chrome) = chrome else {
        eprintln!("remote_browser_stream: no Chromium found, skipping");
        return;
    };
    let home_a = TempHome::new("browser-a");
    let home_b = TempHome::with_config("browser-b", &format!("[browser]\nchromePath = {chrome:?}\n"));
    let a = start(&home_a, "127.0.0.1:0").await;
    let b = start(&home_b, "127.0.0.1:0").await;
    let b_id = b.host_id.clone();
    let mut viewer1 = Client::connect(&a).await;
    let mut viewer2 = Client::connect(&a).await;
    viewer1.call(None, "hosts.add", json!({ "kind": "direct", "url": b.ws_url(), "token": b.token })).await.unwrap();

    let tab = viewer1
        .call(Some(&b_id), "browser.open", json!({ "profile": "default", "url": format!("http://{}/", b.local_addr) }))
        .await
        .unwrap();
    let target_id = tab["targetId"].as_str().unwrap().to_owned();
    let watch = json!({ "profile": "default", "targetId": target_id, "maxWidth": 640, "maxHeight": 480 });
    let is_frame = |e: &Value| {
        e["event"] == json!("browser.frame") && e["host"] == json!(b_id) && e["data"]["targetId"] == json!(target_id)
    };
    viewer1.call(Some(&b_id), "browser.watch", watch.clone()).await.unwrap();
    viewer2.call(Some(&b_id), "browser.watch", watch.clone()).await.unwrap();
    // Force a repaint so frames flow even for a static page.
    viewer1
        .call(Some(&b_id), "browser.resize", json!({ "profile": "default", "targetId": target_id, "width": 600, "height": 400 }))
        .await
        .unwrap();
    viewer1.event("browser.frame (viewer 1)", is_frame).await;
    viewer2.event("browser.frame (viewer 2)", is_frame).await;

    // Viewer 1 leaves; viewer 2 keeps receiving frames from the shared remote stream.
    drop(viewer1);
    tokio::time::sleep(Duration::from_millis(300)).await;
    viewer2.events.clear();
    viewer2
        .call(Some(&b_id), "browser.resize", json!({ "profile": "default", "targetId": target_id, "width": 640, "height": 420 }))
        .await
        .unwrap();
    viewer2.event("browser.frame after viewer 1 left", is_frame).await;

    viewer2.call(Some(&b_id), "browser.unwatch", json!({ "profile": "default", "targetId": target_id })).await.unwrap();
    viewer2.call(Some(&b_id), "browser.stop", json!({ "profile": "default" })).await.unwrap();
    a.shutdown().await;
    b.shutdown().await;
}
