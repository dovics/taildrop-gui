//! Tailscale LocalAPI access: read the tailnet peer list from the local daemon.
//!
//! On Unix this speaks HTTP/1.1 directly over the `tailscaled` unix socket, so
//! it needs no privileged access and no `tailscale` CLI on PATH. On Windows the
//! daemon socket is a named pipe owned by SYSTEM; rather than reimplementing
//! pipe I/O we shell out to `tailscale status --json`, which uses the same pipe.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::time::Duration;

/// One peer on the tailnet, reduced to what a transfer client needs.
#[derive(Debug, Clone)]
pub struct TailnetPeer {
    /// Stable node id ("...CNTRL"). Stable across restarts, unlike the IP.
    pub stable_id: String,
    /// MagicDNS name without the trailing dot, e.g. `desktop.tailxxx.ts.net`.
    pub dns_name: String,
    /// Short display name, e.g. `desktop`.
    pub hostname: String,
    /// `linux`, `windows`, `macOS`, `iOS`, `android`.
    pub os: String,
    pub online: bool,
    /// Tailnet IPv4 (100.64.0.0/10). Present for essentially every node.
    pub ipv4: Option<String>,
    /// Tailnet IPv6 (fd7a:115c:a1e0::/48), used when IPv4 is unavailable.
    pub ipv6: Option<String>,
    /// Direct address in use, e.g. `101.204.221.254:14370`. Empty when relayed.
    pub cur_addr: String,
    /// DERP region code when traffic is relayed, e.g. `sfo`.
    pub relay: String,
    /// The daemon's own verdict on whether this peer accepts Taildrop.
    pub taildrop_target: i32,
    /// Why the peer cannot receive files, empty when it can.
    pub no_file_sharing_reason: String,
    /// Tailscale user id owning the node.
    pub user_id: i64,
    /// Whether this peer is an exit node.
    pub exit_node: bool,
}

impl TailnetPeer {
    /// True when traffic flows peer-to-peer rather than through a DERP relay.
    pub fn is_direct(&self) -> bool {
        !self.cur_addr.is_empty()
    }

    /// Best address to dial: IPv4 first, then IPv6 in brackets.
    pub fn dial_addr(&self) -> Option<String> {
        if let Some(v4) = &self.ipv4 {
            return Some(v4.clone());
        }
        self.ipv6.as_ref().map(|v6| format!("[{v6}]"))
    }

    /// Human-readable connection path for the UI.
    pub fn path_label(&self) -> String {
        if self.is_direct() {
            "direct".to_string()
        } else if !self.relay.is_empty() {
            format!("relay/{}", self.relay)
        } else {
            "unknown".to_string()
        }
    }
}

/// The local node's own identity plus the peer list.
#[derive(Debug, Clone)]
pub struct TailnetSnapshot {
    pub self_hostname: String,
    pub self_dns_name: String,
    pub self_ipv4: Option<String>,
    pub self_user_id: i64,
    pub peers: Vec<TailnetPeer>,
}

impl TailnetSnapshot {
    pub fn online_peers(&self) -> impl Iterator<Item = &TailnetPeer> {
        self.peers.iter().filter(|p| p.online)
    }

    /// Peers owned by the same Tailscale user as this node.
    pub fn same_user_peers(&self) -> impl Iterator<Item = &TailnetPeer> {
        let me = self.self_user_id;
        self.peers.iter().filter(move |p| p.user_id == me)
    }
}

// ---------------------------------------------------------------------------
// raw status JSON (only the fields we read; the real payload is much larger)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawStatus {
    #[serde(rename = "Self")]
    self_node: Option<RawNode>,
    #[serde(rename = "Peer", default)]
    peer: Option<std::collections::HashMap<String, RawNode>>,
}

#[derive(Debug, Deserialize)]
struct RawNode {
    #[serde(rename = "ID", default)]
    id: String,
    #[serde(rename = "StableID", default)]
    stable_id: String,
    #[serde(rename = "HostName", default)]
    hostname: String,
    #[serde(rename = "DNSName", default)]
    dns_name: String,
    #[serde(rename = "OS", default)]
    os: String,
    #[serde(rename = "UserID", default)]
    user_id: i64,
    #[serde(rename = "TailscaleIPs", default)]
    tailscale_ips: Vec<String>,
    #[serde(rename = "Online", default)]
    online: bool,
    #[serde(rename = "CurAddr", default)]
    cur_addr: String,
    #[serde(rename = "Relay", default)]
    relay: String,
    #[serde(rename = "TaildropTarget", default)]
    taildrop_target: i32,
    #[serde(rename = "NoFileSharingReason", default)]
    no_file_sharing_reason: String,
    #[serde(rename = "ExitNode", default)]
    exit_node: bool,
}

fn split_ips(ips: &[String]) -> (Option<String>, Option<String>) {
    let mut v4 = None;
    let mut v6 = None;
    for ip in ips {
        // 100.64.0.0/10 holds the CGNAT range Tailscale assigns for v4.
        if ip.contains(':') {
            if v6.is_none() {
                v6 = Some(ip.clone());
            }
        } else if v4.is_none() {
            v4 = Some(ip.clone());
        }
    }
    (v4, v6)
}

fn trim_dot(mut s: String) -> String {
    if s.ends_with('.') {
        s.pop();
    }
    s
}

fn parse_status(json: &str) -> Result<TailnetSnapshot> {
    let raw: RawStatus = serde_json::from_str(json).context("parsing tailscaled status JSON")?;
    let me = raw.self_node.context("status JSON has no Self node")?;
    let (self_v4, _self_v6) = split_ips(&me.tailscale_ips);

    let mut peers: Vec<TailnetPeer> = raw
        .peer
        .unwrap_or_default()
        .into_values()
        .map(|n| {
            let (v4, v6) = split_ips(&n.tailscale_ips);
            TailnetPeer {
                stable_id: if n.stable_id.is_empty() { n.id } else { n.stable_id },
                dns_name: trim_dot(n.dns_name),
                hostname: n.hostname,
                os: n.os,
                online: n.online,
                ipv4: v4,
                ipv6: v6,
                cur_addr: n.cur_addr,
                relay: n.relay,
                taildrop_target: n.taildrop_target,
                no_file_sharing_reason: n.no_file_sharing_reason,
                user_id: n.user_id,
                exit_node: n.exit_node,
            }
        })
        .collect();

    // Deterministic order: online first, then by hostname.
    peers.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| a.hostname.cmp(&b.hostname)));

    Ok(TailnetSnapshot {
        self_hostname: me.hostname,
        self_dns_name: trim_dot(me.dns_name),
        self_ipv4: self_v4,
        self_user_id: me.user_id,
        peers,
    })
}

// ---------------------------------------------------------------------------
// Unix: HTTP over the tailscaled unix socket
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn default_socket_paths() -> Vec<String> {
    let mut v = Vec::new();
    // macOS installs (both App Store and Developer ID) use this name.
    v.push("/var/run/tailscaled.socket".to_string());
    // Linux, and macOS installs running tailscaled as a daemon.
    v.push("/var/run/tailscale/tailscaled.sock".to_string());
    if let Ok(dir) = std::env::var("TAILSCALE_SOCKET") {
        v.insert(0, dir);
    }
    v
}

#[cfg(unix)]
async fn localapi_get(socket: &str, endpoint: &str) -> Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    let mut stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connecting to tailscaled socket {socket}"))?;

    // The daemon dispatches on the Host header, not the request path alone:
    // a plain "GET /localapi/v0/status" answers "invalid localapi request".
    let req = format!(
        "GET /localapi/v0/{endpoint} HTTP/1.1\r\n\
         Host: local-tailscaled.sock\r\n\
         Connection: close\r\n\
         \r\n"
    );
    stream.write_all(req.as_bytes()).await?;
    stream.flush().await?;

    let mut buf = Vec::with_capacity(64 * 1024);
    stream.read_to_end(&mut buf).await?;
    let text = String::from_utf8_lossy(&buf).to_string();

    let (head, body) = text
        .split_once("\r\n\r\n")
        .context("tailscaled response has no header terminator")?;

    let status_line = head.lines().next().unwrap_or_default();
    if !status_line.contains(" 200 ") {
        bail!("tailscaled returned: {status_line}");
    }

    let chunked = head
        .lines()
        .any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked"));

    if chunked {
        Ok(decode_chunked(body)?)
    } else {
        Ok(body.to_string())
    }
}

/// Minimal `Transfer-Encoding: chunked` decoder; tailscaled always uses it.
fn decode_chunked(body: &str) -> Result<String> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let line_end = match bytes[pos..].windows(2).position(|w| w == b"\r\n") {
            Some(p) => pos + p,
            None => break,
        };
        let size_str = String::from_utf8_lossy(&bytes[pos..line_end]);
        let size = usize::from_str_radix(size_str.trim(), 16)
            .with_context(|| format!("bad chunk size {size_str:?}"))?;
        if size == 0 {
            break;
        }
        let start = line_end + 2;
        let end = start + size;
        if end > bytes.len() {
            bail!("truncated chunked body");
        }
        out.extend_from_slice(&bytes[start..end]);
        pos = end + 2;
    }
    Ok(String::from_utf8_lossy(&out).to_string())
}

#[cfg(unix)]
pub async fn fetch_status() -> Result<TailnetSnapshot> {
    let mut last_err: Option<anyhow::Error> = None;
    for socket in default_socket_paths() {
        if !std::path::Path::new(&socket).exists() {
            continue;
        }
        match localapi_get(&socket, "status").await {
            Ok(body) => return parse_status(&body),
            Err(e) => last_err = Some(e),
        }
    }
    match last_err {
        Some(e) => Err(e).context("no usable tailscaled socket answered"),
        None => bail!(
            "no tailscaled socket found (looked for /var/run/tailscaled.socket, \
             /var/run/tailscale/tailscaled.sock). Is Tailscale running?"
        ),
    }
}

// ---------------------------------------------------------------------------
// Windows: shell out to the CLI, which itself speaks to the SYSTEM pipe
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub async fn fetch_status() -> Result<TailnetSnapshot> {
    // The Windows installer does not put tailscale.exe on PATH, so a bare name
    // fails on a machine where Tailscale is perfectly healthy. Try the default
    // install locations before giving up.
    let candidates: [&str; 2] = [
        "tailscale",
        r"C:\Program Files\Tailscale\tailscale.exe",
    ];
    let mut last_err: Option<String> = None;
    let mut stdout = Vec::new();
    let mut found = false;

    for exe in candidates {
        match tokio::process::Command::new(exe)
            .args(["status", "--json"])
            .output()
            .await
        {
            Ok(out) => {
                if !out.status.success() {
                    last_err = Some(format!(
                        "`tailscale status` failed: {}",
                        String::from_utf8_lossy(&out.stderr)
                    ));
                    continue;
                }
                stdout = out.stdout.to_vec();
                found = true;
                break;
            }
            Err(e) => {
                last_err = Some(format!("{exe}: {e}"));
            }
        }
    }

    if !found {
        bail!(
            "could not run Tailscale ({}). Is it installed?",
            last_err.unwrap_or_else(|| "tailscale not found".into())
        );
    }

    // The CLI may prepend a version-mismatch warning on stderr only, but guard
    // against non-JSON noise on stdout by slicing to the first brace.
    let text = String::from_utf8_lossy(&stdout).to_string();
    let json = match text.find('{') {
        Some(i) => &text[i..],
        None => bail!("`tailscale status --json` printed no JSON"),
    };
    parse_status(json)
}

/// Probe a host's tailnet reachability without touching the daemon.
pub async fn quick_reachable(addr: &str, port: u16, timeout: Duration) -> bool {
    let target = format!("{addr}:{port}");
    matches!(
        tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&target)).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_minimal_status() {
        let json = r#"{
            "Self": {"ID":"n1","StableID":"selfCNTRL","HostName":"me","DNSName":"me.tail.ts.net.",
                     "OS":"linux","UserID":7,"TailscaleIPs":["100.1.1.1","fd7a::1"]},
            "Peer": {
              "a": {"ID":"n2","StableID":"peerCNTRL","HostName":"desk","DNSName":"desk.tail.ts.net.",
                    "OS":"windows","UserID":7,"TailscaleIPs":["100.2.2.2"],
                    "Online":true,"CurAddr":"1.2.3.4:14370","Relay":"sfo",
                    "TaildropTarget":1,"ExitNode":false},
              "b": {"ID":"n3","StableID":"offCNTRL","HostName":"phone","DNSName":"phone.tail.ts.net.",
                    "OS":"iOS","UserID":7,"TailscaleIPs":["100.3.3.3"],
                    "Online":false,"TaildropTarget":5}
            }
        }"#;
        let s = parse_status(json).unwrap();
        assert_eq!(s.self_hostname, "me");
        assert_eq!(s.self_ipv4.as_deref(), Some("100.1.1.1"));
        assert_eq!(s.peers.len(), 2);
        // online peer sorts first
        assert_eq!(s.peers[0].hostname, "desk");
        assert!(s.peers[0].is_direct());
        assert_eq!(s.peers[0].path_label(), "direct");
        assert_eq!(s.peers[0].dial_addr().as_deref(), Some("100.2.2.2"));
        assert_eq!(s.peers[1].path_label(), "unknown");
        assert_eq!(s.same_user_peers().count(), 2);
        assert_eq!(s.online_peers().count(), 1);
    }

    #[test]
    fn decodes_chunked_body() {
        let body = "4\r\n{\"a\"\r\n2\r\n:1\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(body).unwrap(), "{\"a\":1");
    }
}
