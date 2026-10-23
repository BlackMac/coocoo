//! `coocoo` — headless daemon and CLI.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use coocoo_core::{Config, Daemon, DaemonOptions, loopback};
use coocoo_proto::Paths;

#[derive(Parser)]
#[command(name = "coocoo", version, about = "Coocoo agentic development environment — daemon and CLI")]
struct Cli {
    /// Keep config, data and cache under DIR (like `COOCOO_HOME`); lets several daemons share a machine.
    #[arg(long, global = true, value_name = "DIR")]
    home: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the daemon in the foreground (default).
    Serve {
        /// Listen address, overriding `listen` in config.toml (e.g. `0.0.0.0:7421`).
        #[arg(long, value_name = "ADDR")]
        listen: Option<SocketAddr>,
    },
    /// Print this daemon's access token.
    Token,
    /// Print a link + QR code to open the web UI from a phone or another machine.
    Pair {
        /// Address to put in the link (default: first non-loopback IPv4 address).
        #[arg(long, value_name = "HOST")]
        address: Option<String>,
    },
    /// Check whether the daemon is running (GET /api/health); exit status 1 when it is not.
    Status {
        /// Address to probe instead of the running daemon's (or `listen` from config.toml).
        #[arg(long, value_name = "ADDR")]
        listen: Option<SocketAddr>,
    },
    /// Print the version.
    Version,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(home) = &cli.home {
        let home = std::path::absolute(home).unwrap_or_else(|_| home.clone());
        // SAFETY: still single-threaded — no runtime or other threads exist yet. Exported so
        // child processes (omp, and the `coocoo` it may call) agree on the same home.
        unsafe { std::env::set_var("COOCOO_HOME", home) };
    }
    let paths = Paths::from_env();
    let result = match cli.command.unwrap_or(Command::Serve { listen: None }) {
        Command::Serve { listen } => serve(paths, listen),
        Command::Token => token(&paths),
        Command::Pair { address } => pair(&paths, address),
        Command::Status { listen } => status(&paths, listen),
        Command::Version => {
            println!("coocoo {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
    };
    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("coocoo: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn serve(paths: Paths, listen: Option<SocketAddr>) -> anyhow::Result<ExitCode> {
    init_tracing();
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .thread_name("coocoo")
        .enable_all()
        .build()
        .context("starting async runtime")?;
    runtime.block_on(async {
        let daemon = Daemon::start(DaemonOptions { paths, listen }).await?;
        eprintln!("coocoo {} listening on {} (host {})", env!("CARGO_PKG_VERSION"), daemon.local_addr, daemon.host_id);
        if daemon.local_addr.ip().is_loopback() {
            eprintln!("loopback only — run `coocoo pair` for phone/LAN access options");
        }
        wait_for_signal().await;
        daemon.shutdown().await;
        anyhow::Ok(())
    })?;
    // Do not wait for lingering blocking tasks (e.g. a stuck child-process reader).
    runtime.shutdown_timeout(Duration::from_secs(2));
    Ok(ExitCode::SUCCESS)
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = std::env::var("COOCOO_LOG")
        .ok()
        .or_else(|| std::env::var("RUST_LOG").ok())
        .and_then(|spec| EnvFilter::try_new(spec).ok())
        .unwrap_or_else(|| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).with_target(false).init();
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn token(paths: &Paths) -> anyhow::Result<ExitCode> {
    let config = Config::load_or_create(paths)?;
    println!("{}", config.token);
    Ok(ExitCode::SUCCESS)
}

/// Where this home's daemon listens: the address the running daemon recorded (it may have been
/// started with `--listen`), when it answers as this host; else `listen` from config.toml.
fn listen_addr(paths: &Paths, config: &Config) -> SocketAddr {
    std::fs::read_to_string(paths.listen_file())
        .ok()
        .and_then(|s| s.trim().parse::<SocketAddr>().ok())
        .filter(|addr| {
            health(loopback(*addr))
                .ok()
                .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
                .is_some_and(|v| v.get("hostId").and_then(|h| h.as_str()) == Some(config.host_id.as_str()))
        })
        .unwrap_or(config.listen)
}

fn pair(paths: &Paths, address: Option<String>) -> anyhow::Result<ExitCode> {
    let config = Config::load_or_create(paths)?;
    let listen = listen_addr(paths, &config);
    let port = listen.port();
    let host = match address {
        Some(address) => address,
        None => lan_ipv4().map_or_else(|| "127.0.0.1".to_owned(), |ip| ip.to_string()),
    };
    let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host };
    let url = format!("http://{host}:{port}/#token={}", config.token);
    let qr = qrcode::QrCode::new(url.as_bytes()).context("encoding QR code")?;
    let art = qr
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Light)
        .light_color(qrcode::render::unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    println!("{art}\n");
    println!("Open on your phone or another machine:\n  {url}\n");
    if listen.ip().is_loopback() {
        eprintln!(
            "warning: this daemon listens on {} (loopback only), so the link is unreachable from other devices.\n\
             Restart it with `coocoo serve --listen 0.0.0.0:{port}` (or set `listen` in {}),\n\
             or reach it through SSH: `ssh -L {port}:127.0.0.1:{port} <this-host>` and open http://127.0.0.1:{port}/#token=…",
            listen,
            Config::path(paths).display()
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// First non-loopback IPv4 address: the source address the OS would use for an outbound
/// route (a UDP "connect" sends no packets).
fn lan_ipv4() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn status(paths: &Paths, listen: Option<SocketAddr>) -> anyhow::Result<ExitCode> {
    let config = Config::load_or_create(paths)?;
    let addr = loopback(listen.unwrap_or_else(|| listen_addr(paths, &config)));
    match health(addr) {
        Ok(body) => {
            let value: serde_json::Value = serde_json::from_str(&body).context("invalid /api/health response")?;
            let field = |k: &str| value.get(k).and_then(|v| v.as_str()).unwrap_or("?").to_owned();
            println!("running on {addr}: {} (host {}, version {})", field("name"), field("hostId"), field("version"));
            if value.get("hostId").and_then(|v| v.as_str()) != Some(config.host_id.as_str()) {
                eprintln!("warning: {addr} is served by a different coocoo home than {}", paths.config_dir.display());
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(err) => {
            println!("not running on {addr} ({err:#})");
            Ok(ExitCode::FAILURE)
        }
    }
}

/// Minimal blocking `GET /api/health` (no HTTP client dependency for one request).
fn health(addr: SocketAddr) -> anyhow::Result<String> {
    let timeout = Duration::from_secs(3);
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write!(stream, "GET /api/health HTTP/1.1\r\nHost: {addr}\r\nAccept: application/json\r\nConnection: close\r\n\r\n")?;
    let mut response = String::new();
    stream.take(64 * 1024).read_to_string(&mut response)?;
    let (head, body) = response.split_once("\r\n\r\n").context("malformed HTTP response")?;
    let status = head.split_whitespace().nth(1).unwrap_or_default();
    anyhow::ensure!(status == "200", "HTTP status {status}");
    Ok(body.to_owned())
}
