//! Taildrop CLI: list your tailnet, probe for peers running a transfer server,
//! and send files to one of them.

use anyhow::{bail, Context, Result};
use taildrop_core::localsend::identity::Identity;
use taildrop_core::localsend::sender::{self, Endpoint, OutgoingFile};
use taildrop_core::localsend::types::{DeviceInfo, DeviceType, Protocol};
use taildrop_core::localsend::{DEFAULT_PORT, PROTOCOL_VERSION};
use taildrop_core::tailscale::{self, TailnetPeer, TailnetSnapshot};
use clap::{Parser, Subcommand};
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "taildrop-cli", version, about = "Send files over your tailnet")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show tailnet peers and which of them run a transfer server.
    Peers {
        /// Port to probe (Taildrop and LocalSend defaults are both tried).
        #[arg(long)]
        port: Option<u16>,
        /// Include offline peers.
        #[arg(long)]
        all: bool,
        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
        /// Per-probe timeout in milliseconds.
        #[arg(long, default_value_t = 800)]
        timeout_ms: u64,
    },
    /// Send files (or a text snippet) to a peer.
    Send {
        /// Files to send.
        files: Vec<String>,
        /// Destination: a peer hostname, DNS name, or literal tailnet IP.
        #[arg(short, long)]
        to: String,
        /// Override the destination port.
        #[arg(long)]
        port: Option<u16>,
        /// Transport: auto (default), http, or https.
        #[arg(long, default_value = "auto")]
        tls: String,
        /// Receiver PIN, if one is configured.
        #[arg(long)]
        pin: Option<String>,
        /// Required fingerprint (uppercase hex). Refuses a mismatch.
        #[arg(long)]
        expect_fingerprint: Option<String>,
    },
    /// Run the receiver: save incoming files into a directory.
    Serve {
        /// Directory to save files into.
        #[arg(short, long)]
        inbox: Option<String>,
        /// Port to listen on.
        #[arg(long)]
        port: Option<u16>,
        /// Require this PIN from senders.
        #[arg(long)]
        pin: Option<String>,
        /// Emit one JSON object per line instead of human text.
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Must precede any TLS work: see taildrop_core::init_crypto.
    taildrop_core::init_crypto();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Peers {
            port,
            all,
            json,
            timeout_ms,
        } => cmd_peers(port, all, json, timeout_ms).await,
        Cmd::Send {
            files,
            to,
            port,
            tls,
            pin,
            expect_fingerprint,
        } => cmd_send(files, to, port, tls, pin, expect_fingerprint).await,
        Cmd::Serve { inbox, port, pin, json } => cmd_serve(inbox, port, pin, json).await,
    }
}

async fn cmd_serve(
    inbox: Option<String>,
    port: Option<u16>,
    pin: Option<String>,
    json: bool,
) -> Result<()> {
    use taildrop_core::localsend::receiver::{self, ReceivedFile, ServerConfig};

    let alias = hostname();
    let identity = Identity::load_or_create(&Identity::default_dir(), &alias)?;
    let inbox_path = match inbox {
        Some(p) => std::path::PathBuf::from(p),
        None => dirs_downloads().join("Taildrop"),
    };
    let listen_port = port.unwrap_or(DEFAULT_PORT);

    if !json {
        println!("taildrop-cli receiver");
        println!("  alias:       {alias}");
        println!("  fingerprint: {}", identity.fingerprint);
        println!("  inbox:       {}", inbox_path.display());
    }

    let fingerprint = identity.fingerprint.clone();
    let on_received = std::sync::Arc::new(move |f: ReceivedFile| {
        if json {
            // One object per line: the UI reads events, never scraped text.
            let event = serde_json::json!({
                "event": "received",
                "fileName": f.file_name,
                "path": f.saved_path.to_string_lossy(),
                "size": f.size,
                "fromAlias": f.from_alias,
                "fromAddr": f.from_addr,
                "verified": f.verified,
                "elapsedMs": f.elapsed.as_millis() as u64,
            });
            println!("{event}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
        } else {
            println!(
                "✓ {} from {} ({}) → {}",
                human_bytes(f.size),
                if f.from_alias.is_empty() { "unknown" } else { &f.from_alias },
                if f.verified { "verified" } else { "unverified" },
                f.saved_path.display()
            );
        }
    });

    let handle = receiver::start(ServerConfig {
        identity,
        inbox: inbox_path.clone(),
        port: listen_port,
        pin,
        policy: None,
        on_received: Some(on_received),
    })
    .await?;

    if json {
        let event = serde_json::json!({
            "event": "ready",
            "alias": alias,
            "fingerprint": fingerprint,
            "inbox": inbox_path.to_string_lossy(),
            "port": handle.local_addr.port(),
            "addr": handle.local_addr.to_string(),
        });
        println!("{event}");
        use std::io::Write;
        let _ = std::io::stdout().flush();
    } else {
        println!("  listening:   {}", handle.local_addr);
        println!();
        println!("ready — press Ctrl-C to stop");
    }

    tokio::signal::ctrl_c().await.ok();
    handle.shutdown();
    if !json {
        println!("stopped");
    }
    Ok(())
}

fn dirs_downloads() -> std::path::PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        for name in ["Downloads", "下载"] {
            let p = home.join(name);
            if p.is_dir() {
                return p;
            }
        }
        return home.join("Downloads");
    }
    std::path::PathBuf::from(".")
}

/// Our own advertised identity, shared by probe and send.
fn local_info(identity: &Identity) -> DeviceInfo {
    DeviceInfo {
        alias: hostname(),
        version: PROTOCOL_VERSION.to_string(),
        device_model: None,
        device_type: Some(DeviceType::Headless),
        fingerprint: identity.fingerprint.clone(),
        port: DEFAULT_PORT,
        protocol: Protocol::Https,
        download: false,
    }
}

fn hostname() -> String {
    std::env::var("TAILDROP_ALIAS")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "taildrop-gui".to_string())
}

/// What a probe learned about one peer.
#[derive(Clone)]
struct Found {
    peer: TailnetPeer,
    port: u16,
    protocol: Protocol,
    /// Fingerprint the peer reports (empty when it answered 204).
    fingerprint: String,
    alias: String,
    latency: Duration,
}

async fn cmd_peers(
    port: Option<u16>,
    all: bool,
    json: bool,
    timeout_ms: u64,
) -> Result<()> {
    let snap = tailscale::fetch_status()
        .await
        .context("reading the tailnet device list from Tailscale")?;

    let candidates: Vec<TailnetPeer> = if all {
        snap.peers.clone()
    } else {
        snap.online_peers().cloned().collect()
    };

    let identity = Identity::load_or_create(&Identity::default_dir(), &hostname())?;
    let client = sender::build_client(&identity)?;
    let info = local_info(&identity);

    // Taildrop's port first, then LocalSend's, so an unmodified LocalSend peer is
    // still discovered.
    let ports: Vec<u16> = match port {
        Some(p) => vec![p],
        None => vec![DEFAULT_PORT, 53317],
    };
    let timeout = Duration::from_millis(timeout_ms);

    let mut tasks = Vec::new();
    for peer in candidates.iter() {
        for p in &ports {
            let addr = match peer.dial_addr() {
                Some(a) => a,
                None => continue,
            };
            let client = client.clone();
            let info = info.clone();
            let peer_clone = peer.clone();
            let p = *p;
            let online = peer.online;
            tasks.push(tokio::spawn(async move {
                let started = Instant::now();
                // Try HTTPS (mTLS) first: that is what a default LocalSend or
                // Taildrop server speaks. Fall back to plain HTTP for peers that
                // disabled encryption.
                for proto in [Protocol::Https, Protocol::Http] {
                    let ep = Endpoint::new(addr.clone(), p, proto);
                    if let Ok(resp) = sender::probe(&client, &ep, &info, timeout).await {
                        return Some(Found {
                            peer: peer_clone,
                            port: p,
                            protocol: proto,
                            fingerprint: resp.fingerprint,
                            alias: resp.alias,
                            latency: started.elapsed(),
                        });
                    }
                    if !online {
                        break;
                    }
                }
                None
            }));
        }
    }

    // A peer can answer on more than one port (Taildrop and LocalSend both
    // running, say). Keep the fastest answer per peer.
    let mut best: std::collections::HashMap<String, Found> = std::collections::HashMap::new();
    for t in tasks {
        if let Ok(Some(f)) = t.await {
            let key = f.peer.stable_id.clone();
            match best.get(&key) {
                Some(existing) if existing.latency <= f.latency => {}
                _ => {
                    best.insert(key, f);
                }
            }
        }
    }
    let mut found: Vec<Found> = best.into_values().collect();
    found.sort_by(|a, b| a.peer.hostname.cmp(&b.peer.hostname));

    if json {
        let rows: Vec<_> = candidates
            .iter()
            .map(|p| {
                let f = found.iter().find(|f| f.peer.stable_id == p.stable_id);
                serde_json::json!({
                    "hostname": p.hostname,
                    "dnsName": p.dns_name,
                    "os": p.os,
                    "online": p.online,
                    "ipv4": p.ipv4,
                    "path": p.path_label(),
                    "taildropTarget": p.taildrop_target,
                    "transferServer": f.map(|f| serde_json::json!({
                        "port": f.port,
                        "protocol": f.protocol.scheme(),
                        "alias": f.alias,
                        "fingerprint": f.fingerprint,
                        "latencyMs": f.latency.as_millis() as u64,
                    })),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("this device: {} ({})", snap.self_hostname, snap.self_dns_name);
    if let Some(ip) = &snap.self_ipv4 {
        println!("tailnet ip:  {ip}");
    }
    println!();

    if candidates.is_empty() {
        println!("no peers to show (use --all to include offline ones)");
        return Ok(());
    }

    println!(
        "{:<22} {:<8} {:<8} {:<16} {:<10} {}",
        "PEER", "OS", "PATH", "IP", "LATENCY", "TRANSFER"
    );
    for peer in &candidates {
        let hit = found.iter().find(|f| f.peer.stable_id == peer.stable_id);
        let (latency, transfer) = match hit {
            Some(f) => (
                format!("{}ms", f.latency.as_millis()),
                format!(
                    "✓ {}:{} {}",
                    f.protocol.scheme(),
                    f.port,
                    if f.alias.is_empty() { "" } else { &f.alias }
                ),
            ),
            None => ("-".to_string(), if peer.online { "not found".into() } else { "offline".into() }),
        };
        println!(
            "{:<22} {:<8} {:<8} {:<16} {:<10} {}",
            truncate(&peer.hostname, 22),
            truncate(&peer.os, 8),
            peer.path_label(),
            peer.ipv4.clone().unwrap_or_else(|| "-".into()),
            latency,
            transfer
        );
    }

    let ready = found.len();
    println!();
    println!("{ready} of {} peer(s) accept transfers", candidates.len());
    if ready == 0 && !candidates.is_empty() {
        println!("tip: peers need Taildrop or LocalSend running to receive files");
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Resolve a user-supplied destination to an endpoint, consulting the tailnet
/// list so a hostname works instead of an IP.
async fn resolve(
    to: &str,
    port: Option<u16>,
    tls: &str,
    snap: Option<&TailnetSnapshot>,
) -> Result<Endpoint> {
    let protocol = match tls {
        "http" => Protocol::Http,
        "https" => Protocol::Https,
        "auto" => Protocol::Https,
        other => bail!("--tls must be auto, http or https (got {other})"),
    };
    let default_port = port.unwrap_or(DEFAULT_PORT);

    // Already an address?
    let looks_like_ip = to.parse::<std::net::Ipv4Addr>().is_ok()
        || (to.contains(':') && to.parse::<std::net::Ipv6Addr>().is_ok());
    if looks_like_ip {
        let host = if to.contains(':') && !to.starts_with('[') {
            format!("[{to}]")
        } else {
            to.to_string()
        };
        return Ok(Endpoint::new(host, default_port, protocol));
    }

    // A peer's own short hostname wins over a tailnet name that merely looks
    // like one: an iPhone whose hostname is "localhost" must not swallow
    // `--to localhost`, which is how a caller says "this machine".
    if to.eq_ignore_ascii_case("localhost") || to == "127.0.0.1" {
        return Ok(Endpoint::new("127.0.0.1", default_port, protocol));
    }

    // Otherwise match against known peers by hostname, short DNS name, or FQDN.
    let needle = to.trim_end_matches('.').to_ascii_lowercase();
    let found = snap.and_then(|s| {
        s.peers.iter().find(|p| {
            p.hostname.eq_ignore_ascii_case(&needle)
                || p.dns_name.eq_ignore_ascii_case(&needle)
                || p
                    .dns_name
                    .split('.')
                    .next()
                    .map(|s| s.eq_ignore_ascii_case(&needle))
                    .unwrap_or(false)
        })
    });

    match found {
        Some(peer) => {
            let addr = peer
                .dial_addr()
                .with_context(|| format!("peer {} has no tailnet address", peer.hostname))?;
            Ok(Endpoint::new(addr, default_port, protocol))
        }
        None => bail!(
            "no peer named {to:?} on your tailnet. Run `taildrop-cli peers` to see who is there, \
             or pass a literal tailnet IP."
        ),
    }
}

async fn cmd_send(
    files: Vec<String>,
    to: String,
    port: Option<u16>,
    tls: String,
    pin: Option<String>,
    expect_fingerprint: Option<String>,
) -> Result<()> {
    if files.is_empty() {
        bail!("no files given");
    }

    let snap = tailscale::fetch_status().await.ok();
    let endpoint = resolve(&to, port, &tls, snap.as_ref()).await?;

    let identity = Identity::load_or_create(&Identity::default_dir(), &hostname())?;
    let client = sender::build_client(&identity)?;
    let info = local_info(&identity);

    // Probe first: learn the peer's identity, then decide whether to trust it.
    // A silent peer is fine to send to — the receiver may simply not answer
    // register — but then we have no fingerprint to check.
    let probe = sender::probe(&client, &endpoint, &info, Duration::from_secs(3))
        .await
        .ok();

    if let Some(p) = &probe {
        let claimed = &p.fingerprint;
        sender::verify_peer_fingerprint(claimed, expect_fingerprint.as_deref())?;
        println!(
            "→ {} at {} ({})",
            if p.alias.is_empty() { &to } else { &p.alias },
            endpoint.base_url(),
            endpoint.protocol.scheme()
        );
    } else if expect_fingerprint.is_some() {
        bail!("asked to verify a fingerprint, but the peer did not answer a probe");
    } else {
        println!("→ {} at {}", to, endpoint.base_url());
    }

    let mut staged = Vec::new();
    for path in &files {
        let f = OutgoingFile::stage(path)
            .await
            .with_context(|| format!("preparing {path}"))?;
        staged.push(f);
    }
    let total: u64 = staged.iter().map(|f| f.size).sum();
    println!(
        "  {} file(s), {}",
        staged.len(),
        human_bytes(total)
    );

    let progress = std::sync::Arc::new(|done: u64, total: u64| {
        if total == 0 {
            return;
        }
        let pct = (done as f64 / total as f64 * 100.0).min(100.0);
        let width = 28usize;
        let filled = (pct / 100.0 * width as f64) as usize;
        print!(
            "\r  [{}{}] {:>3.0}%  {}/{}",
            "█".repeat(filled),
            "░".repeat(width - filled),
            pct,
            human_bytes(done),
            human_bytes(total)
        );
        use std::io::Write;
        let _ = std::io::stdout().flush();
    });

    let outcome = sender::send_files(
        &identity,
        &endpoint,
        &info,
        &staged,
        pin.as_deref(),
        Some(progress),
    )
    .await;

    match outcome {
        Ok(o) => {
            println!();
            println!(
                "✓ sent {} file(s), {} in {:.1}s ({:.1} Mbit/s)",
                o.files_sent,
                human_bytes(o.bytes_sent),
                o.elapsed.as_secs_f64(),
                o.throughput_mbps()
            );
            Ok(())
        }
        Err(e) => {
            println!();
            Err(e)
        }
    }
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}
