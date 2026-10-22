use std::time::{Duration, Instant};

use coocoo_browser::{Browser, BrowserConfig};
use coocoo_proto::{ConnSink, Emitter, Paths};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

const PAGE: &str = "<!doctype html><title>static</title><body style='margin:0'><h1 style='width:50%'>Static</h1><p>text</p></body>";

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf).await;
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{PAGE}", PAGE.len());
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    let root = std::env::temp_dir().join(format!("coocoo-rprobe-{}", std::process::id()));
    let browser = Browser::new(
        Emitter::new("h", 64),
        &Paths::under(root.clone()),
        BrowserConfig { chrome_path: Some("/usr/bin/google-chrome".into()), ..Default::default() },
        "http://127.0.0.1:1".into(),
    );
    let (tx, mut rx) = mpsc::channel(64);
    let conn = ConnSink::new(1, tx);
    let tab = browser.dispatch("browser.open", json!({"profile":"p","url":format!("http://127.0.0.1:{port}/")}), &conn).await.unwrap();
    let t = tab["targetId"].as_str().unwrap().to_owned();
    browser.dispatch("browser.watch", json!({"profile":"p","targetId":t,"maxWidth":640,"maxHeight":480}), &conn).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut misses = 0;
    for i in 0..30u32 {
        while rx.try_recv().is_ok() {}
        let (w, h) = if i % 2 == 0 { (600, 400) } else { (640, 420) };
        let start = Instant::now();
        browser.dispatch("browser.resize", json!({"profile":"p","targetId":t,"width":w,"height":h}), &conn).await.unwrap();
        let got = loop {
            match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
                Ok(Some(m)) => {
                    let v: Value = serde_json::from_str(&m).unwrap();
                    if v["event"] == "browser.frame" {
                        break Some((v["data"]["width"].clone(), v["data"]["height"].clone()));
                    }
                }
                _ => break None,
            }
        };
        if got.is_none() {
            misses += 1;
        }
        println!("resize {i} {w}x{h} -> {got:?} in {:?}", start.elapsed());
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    println!("MISSES {misses}/30");
    browser.shutdown().await;
    let _ = std::fs::remove_dir_all(root);
}
