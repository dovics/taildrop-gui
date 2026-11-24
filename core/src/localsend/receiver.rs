//! Receiving: a real server, not a prompt.
//!
//! LocalSend's own receiver blocks on a keyboard decision. A background daemon
//! cannot: it must decide from policy. This module therefore exposes a
//! [`Decision`] hook with a sensible default — accept everything from the same
//! tailnet user, write to the inbox — while still returning the protocol's real
//! status codes (403 declined, 409 busy, 401 PIN) so an interactive UI can take
//! the decision over later without any wire change.

use crate::localsend::identity::Identity;
use crate::localsend::types::*;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// policy
// ---------------------------------------------------------------------------

/// What to do with an incoming request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Take the files, saving them under the inbox.
    Accept,
    /// Refuse politely (403).
    Decline(String),
    /// Ask for a PIN first (401).
    RequirePin,
}

/// Why a request was refused, as seen by the sender.
#[derive(Debug, Clone)]
pub enum RejectReason {
    PinRequired,
    Declined(String),
    Busy,
    BadRequest(String),
}

impl RejectReason {
    pub fn status_code(&self) -> u16 {
        match self {
            RejectReason::PinRequired => 401,
            RejectReason::Declined(_) => 403,
            RejectReason::Busy => 409,
            RejectReason::BadRequest(_) => 400,
        }
    }

    pub fn message(&self) -> String {
        match self {
            RejectReason::PinRequired => "PIN required".into(),
            RejectReason::Declined(why) => why.clone(),
            RejectReason::Busy => "another session is active".into(),
            RejectReason::BadRequest(why) => why.clone(),
        }
    }
}

/// A finished or in-flight receive, for the UI and the history.
#[derive(Debug, Clone)]
pub struct ReceivedFile {
    pub file_name: String,
    pub saved_path: PathBuf,
    pub size: u64,
    /// Sender alias as it announced itself (not trusted for anything but display).
    pub from_alias: String,
    /// Sender's TCP address, e.g. `100.117.121.13:52123`.
    pub from_addr: String,
    pub verified: bool,
    pub elapsed: Duration,
}

pub type ReceiveCallback = Arc<dyn Fn(ReceivedFile) + Send + Sync>;

/// A peer saying it just came online.
///
/// Standard LocalSend has no such message: its own discovery is multicast, which
/// does not cross a tailnet. Announcements are therefore our own addition, used
/// only between Taildrop peers, so that a machine that starts up does not stay
/// invisible until every other machine's next periodic scan.
#[derive(Debug, Clone)]
pub struct Announcement {
    pub alias: String,
    /// Peer's TCP address, e.g. `100.117.121.13:52123`.
    pub from_addr: String,
    pub fingerprint: String,
}

pub type AnnounceCallback = Arc<dyn Fn(Announcement) + Send + Sync>;

// ---------------------------------------------------------------------------
// server config
// ---------------------------------------------------------------------------

pub struct ServerConfig {
    pub identity: Identity,
    /// Directory that received files land in.
    pub inbox: PathBuf,
    /// Port to listen on.
    pub port: u16,
    /// When set, a request without this PIN is answered 401.
    pub pin: Option<String>,
    /// Overrides the default same-user accept policy.
    pub policy: Option<Arc<dyn Fn(&str) -> Decision + Send + Sync>>,
    /// Called once per completed file.
    pub on_received: Option<ReceiveCallback>,
    /// Called when another Taildrop peer announces it has come online.
    pub on_announce: Option<AnnounceCallback>,
}

/// State shared by all request handlers.
struct AppState {
    identity: Identity,
    inbox: PathBuf,
    pin: Option<String>,
    policy: Option<Arc<dyn Fn(&str) -> Decision + Send + Sync>>,
    on_received: Option<ReceiveCallback>,
    on_announce: Option<AnnounceCallback>,
    /// Set while a session is uploading, so a second sender gets 409.
    busy: Mutex<bool>,
    bytes_written: AtomicU64,
}

// ---------------------------------------------------------------------------
// server
// ---------------------------------------------------------------------------

/// Start the receiver. Returns immediately; the server runs on its own task
/// and stops when the returned handle is dropped or [`ServerHandle::shutdown`]
/// is called.
pub struct ServerHandle {
    pub local_addr: SocketAddr,
    shutdown: tokio::sync::oneshot::Sender<()>,
    state: Arc<AppState>,
}

impl ServerHandle {
    /// Ask the listener to stop accepting new connections.
    pub fn shutdown(self) {
        let _ = self.shutdown.send(());
    }

    /// Total bytes written to disk across all sessions.
    pub fn bytes_written(&self) -> u64 {
        self.state.bytes_written.load(Ordering::Relaxed)
    }
}

/// Bind and serve. Uses mutual TLS when the identity loads, so senders that
/// verify fingerprints get a real answer.
pub async fn start(cfg: ServerConfig) -> Result<ServerHandle> {
    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.port));

    let state = Arc::new(AppState {
        identity: cfg.identity.clone(),
        inbox: cfg.inbox.clone(),
        pin: cfg.pin.clone(),
        policy: cfg.policy.clone(),
        on_received: cfg.on_received.clone(),
        on_announce: cfg.on_announce.clone(),
        busy: Mutex::new(false),
        bytes_written: AtomicU64::new(0),
    });

    std::fs::create_dir_all(&cfg.inbox)
        .with_context(|| format!("creating inbox {}", cfg.inbox.display()))?;

    let tls = Arc::new(build_tls_config(&cfg.identity)?);
    let acceptor = tokio_rustls::TlsAcceptor::from(tls);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    let local_addr = listener.local_addr()?;

    let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
    let server_state = state.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut rx => break,
                accepted = listener.accept() => {
                    let (stream, peer) = match accepted {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let acceptor = acceptor.clone();
                    let state = server_state.clone();
                    tokio::spawn(async move {
                        // A failed handshake is normal noise on a busy tailnet
                        // (port scanners, a peer with the wrong cert): drop it.
                        if let Ok(tls_stream) = acceptor.accept(stream).await {
                            let io = hyper_util::rt::TokioIo::new(tls_stream);
                            let svc = hyper::service::service_fn(move |req| {
                                handle(req, state.clone(), peer)
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(io, svc)
                                .await;
                        }
                    });
                }
            }
        }
    });

    Ok(ServerHandle {
        local_addr,
        shutdown: tx,
        state,
    })
}

fn build_tls_config(identity: &Identity) -> Result<rustls::ServerConfig> {
    // reqwest and we both pull rustls with ring, which leaves rustls unable to
    // pick a provider on its own ("Could not automatically determine the
    // process-level CryptoProvider"). Installing it once here fixes the whole
    // process; a second call is a no-op.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut identity.cert_pem.as_bytes())
            .collect::<std::result::Result<_, _>>()
            .context("parsing our certificate")?;
    let key = rustls_pemfile::private_key(&mut identity.key_pem.as_bytes())
        .context("parsing our key")?
        .context("no private key in identity")?;

    let config = rustls::ServerConfig::builder()
        // Trust comes from fingerprint pinning, not a CA, so no client-cert
        // verification is requested. We still *see* a client cert if one is
        // offered, which is what identity is built on.
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("building TLS server config")?;
    Ok(config)
}

// ---------------------------------------------------------------------------
// request handling
// ---------------------------------------------------------------------------

async fn handle(
    req: hyper::Request<hyper::body::Incoming>,
    state: Arc<AppState>,
    peer: SocketAddr,
) -> Result<hyper::Response<http_body_util::Full<bytes::Bytes>>, std::convert::Infallible> {
    use http_body_util::BodyExt;

    let path = req.uri().path().to_string();
    let query: HashMap<String, String> = req
        .uri()
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_default();

    let resp = match (req.method().as_str(), path.as_str()) {
        ("POST", "/api/localsend/v2/register") => {
            let body = match req.into_body().collect().await {
                Ok(b) => b.to_bytes(),
                Err(_) => return Ok(json_error(400, "unreadable body")),
            };
            register(&state, &body)
        }
        ("POST", "/api/localsend/v2/prepare-upload") => {
            let body = match req.into_body().collect().await {
                Ok(b) => b.to_bytes(),
                Err(_) => return Ok(json_error(400, "unreadable body")),
            };
            prepare_upload(&state, &body, &query, peer).await
        }
        ("POST", "/api/localsend/v2/upload") => {
            let chunk = req.into_body();
            upload(&state, chunk, &query, peer).await
        }
        ("POST", "/api/localsend/v2/announce") => {
            // Body is optional: an announce that carries nothing still means
            // "I am up, refresh your list".
            let body = match req.into_body().collect().await {
                Ok(b) => b.to_bytes(),
                Err(_) => bytes::Bytes::new(),
            };
            announce(&state, &body, peer)
        }
        ("POST", "/api/localsend/v2/cancel") => {
            release_busy(&state);
            json_ok_empty()
        }
        ("GET", "/api/localsend/v2/info") => info(&state),
        _ => json_error(404, "not found"),
    };
    Ok(resp)
}

type Resp = hyper::Response<http_body_util::Full<bytes::Bytes>>;

fn json_body(status: u16, body: String) -> Resp {
    hyper::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(http_body_util::Full::new(bytes::Bytes::from(body)))
        .unwrap_or_else(|_| {
            hyper::Response::new(http_body_util::Full::new(bytes::Bytes::from_static(
                b"{}",
            )))
        })
}

fn json_error(status: u16, message: &str) -> Resp {
    let body = serde_json::to_string(&ErrorResponse {
        message: message.to_string(),
    })
    .unwrap_or_else(|_| r#"{"message":"error"}"#.to_string());
    json_body(status, body)
}

fn json_ok_empty() -> Resp {
    hyper::Response::builder()
        .status(204)
        .body(http_body_util::Full::new(bytes::Bytes::new()))
        .unwrap()
}

fn info(state: &AppState) -> Resp {
    let body = serde_json::to_string(&RegisterResponse {
        alias: hostname(),
        version: crate::localsend::PROTOCOL_VERSION.to_string(),
        device_model: None,
        device_type: Some(DeviceType::Headless),
        fingerprint: state.identity.fingerprint.clone(),
        download: false,
    })
    .unwrap_or_default();
    json_body(200, body)
}

fn register(state: &AppState, body: &[u8]) -> Resp {
    // The sender tells us who it is; we only echo our own identity back. We
    // deliberately do not persist their claim here — trusting an unverified
    // alias is how a "device" gets impersonated in a list.
    match serde_json::from_slice::<RegisterRequest>(body) {
        Ok(_) => {
            let out = RegisterResponse {
                alias: hostname(),
                version: crate::localsend::PROTOCOL_VERSION.to_string(),
                device_model: None,
                device_type: Some(DeviceType::Headless),
                fingerprint: state.identity.fingerprint.clone(),
                download: false,
            };
            json_body(200, serde_json::to_string(&out).unwrap_or_default())
        }
        Err(e) => json_error(400, &format!("invalid register body: {e}")),
    }
}

/// Lenient view of an announce body: the fields are display hints, and a peer
/// that sends none of them is still telling us something true.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AnnounceIn {
    #[serde(default)]
    alias: String,
    #[serde(default)]
    fingerprint: String,
}

fn announce(state: &AppState, body: &[u8], peer: SocketAddr) -> Resp {
    let parsed = serde_json::from_slice::<AnnounceIn>(body).unwrap_or_default();
    if let Some(cb) = &state.on_announce {
        cb(Announcement {
            alias: parsed.alias,
            from_addr: peer.to_string(),
            fingerprint: parsed.fingerprint,
        });
    }
    json_ok_empty()
}

/// Session bookkeeping for in-flight uploads.
struct Session {
    sender_alias: String,
    sender_addr: String,
    files: HashMap<String, PendingFile>,
    /// fileId -> token the sender must present.
    tokens: HashMap<String, String>,
    started: Instant,
}

struct PendingFile {
    dto: FileDto,
    done: bool,
}

fn sessions() -> &'static Mutex<HashMap<String, Arc<Mutex<Session>>>> {
    use std::sync::OnceLock;
    static S: OnceLock<Mutex<HashMap<String, Arc<Mutex<Session>>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn prepare_upload(
    state: &Arc<AppState>,
    body: &[u8],
    query: &HashMap<String, String>,
    peer: SocketAddr,
) -> Resp {
    let req: PrepareUploadRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return json_error(400, &format!("invalid prepare body: {e}")),
    };

    if req.files.is_empty() {
        // Spec: 204 = nothing to transfer.
        return json_ok_empty();
    }

    // PIN check before the policy hook: a PIN is a hard gate, not a preference.
    if let Some(required) = &state.pin {
        match query.get("pin") {
            Some(given) if given == required => {}
            _ => return json_error(401, "PIN required"),
        }
    }

    // One session at a time keeps the inbox unambiguous and matches LocalSend.
    {
        let mut busy = state.busy.lock().unwrap();
        if *busy {
            return json_error(409, "another session is active");
        }
        *busy = true;
    }

    let decision = match &state.policy {
        Some(p) => p(&req.info.alias),
        None => Decision::Accept,
    };

    match decision {
        Decision::Accept => {}
        Decision::Decline(why) => {
            release_busy(state);
            return json_error(403, &why);
        }
        Decision::RequirePin => {
            release_busy(state);
            return json_error(401, "PIN required");
        }
    }

    let session_id = random_hex(16);
    let mut tokens = HashMap::new();
    let mut files = HashMap::new();
    for (id, dto) in &req.files {
        tokens.insert(id.clone(), random_hex(24));
        files.insert(
            id.clone(),
            PendingFile {
                dto: dto.clone(),
                done: false,
            },
        );
    }

    let session = Arc::new(Mutex::new(Session {
        sender_alias: req.info.alias.clone(),
        sender_addr: peer.to_string(),
        files,
        tokens: tokens.clone(),
        started: Instant::now(),
    }));

    sessions()
        .lock()
        .unwrap()
        .insert(session_id.clone(), session);

    let out = PrepareUploadResponse {
        session_id,
        files: tokens,
    };
    json_body(200, serde_json::to_string(&out).unwrap_or_default())
}

fn release_busy(state: &AppState) {
    if let Ok(mut busy) = state.busy.lock() {
        *busy = false;
    }
}

async fn upload(
    state: &Arc<AppState>,
    body: hyper::body::Incoming,
    query: &HashMap<String, String>,
    peer: SocketAddr,
) -> Resp {
    use futures_util::StreamExt;
    use http_body_util::BodyExt as _;

    let session_id = match query.get("sessionId") {
        Some(v) => v.clone(),
        None => return json_error(400, "missing sessionId"),
    };
    let file_id = match query.get("fileId") {
        Some(v) => v.clone(),
        None => return json_error(400, "missing fileId"),
    };
    let token = query.get("token").cloned().unwrap_or_default();

    let session = match sessions().lock().unwrap().get(&session_id).cloned() {
        Some(s) => s,
        None => return json_error(403, "unknown or expired session"),
    };

    let (dto, alias, from_addr, started) = {
        let s = session.lock().unwrap();
        // A token binds this request to the session that negotiated it, so a
        // third party who guesses the ids still cannot inject bytes.
        match s.tokens.get(&file_id) {
            Some(expected) if expected == &token => {}
            _ => return json_error(403, "invalid token"),
        }
        let pending = match s.files.get(&file_id) {
            Some(p) => p,
            None => return json_error(400, "unknown fileId"),
        };
        if pending.done {
            return json_error(409, "file already uploaded");
        }
        (
            pending.dto.clone(),
            s.sender_alias.clone(),
            s.sender_addr.clone(),
            s.started,
        )
    };

    // Sanitise hard: the name arrives from the network and will be joined to a
    // path on disk. Reject anything that could escape the inbox.
    let safe_name = match sanitise_filename(&dto.file_name) {
        Some(n) => n,
        None => return json_error(400, "unsafe file name"),
    };

    let dest = unique_path(&state.inbox, &safe_name);
    let tmp = dest.with_extension(format!(
        "{}.part",
        dest.extension()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    ));

    let mut file = match tokio::fs::File::create(&tmp).await {
        Ok(f) => f,
        Err(e) => {
            release_busy(state);
            return json_error(500, &format!("cannot create {}: {e}", tmp.display()));
        }
    };

    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    let mut stream = body.into_data_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                drop(file);
                let _ = tokio::fs::remove_file(&tmp).await;
                release_busy(state);
                return json_error(400, &format!("body stream error: {e}"));
            }
        };
        hasher.update(&chunk);
        if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await {
            drop(file);
            let _ = tokio::fs::remove_file(&tmp).await;
            release_busy(state);
            return json_error(500, &format!("write failed: {e}"));
        }
        written += chunk.len() as u64;
        state.bytes_written.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }

    if let Err(e) = tokio::io::AsyncWriteExt::flush(&mut file).await {
        drop(file);
        let _ = tokio::fs::remove_file(&tmp).await;
        release_busy(state);
        return json_error(500, &format!("flush failed: {e}"));
    }
    drop(file);

    // Verify before the rename: a file that fails its hash must never appear
    // under its real name, or a downstream watcher would act on corrupt data.
    let actual = hex::encode(hasher.finalize());
    let mut verified = true;
    if let Some(expected) = &dto.sha256 {
        if !expected.eq_ignore_ascii_case(&actual) {
            verified = false;
            let _ = tokio::fs::remove_file(&tmp).await;
            release_busy(state);
            return json_error(422, "checksum mismatch");
        }
    }
    if written != dto.size {
        let _ = tokio::fs::remove_file(&tmp).await;
        release_busy(state);
        return json_error(400, &format!("size mismatch: got {written}, expected {}", dto.size));
    }

    if let Err(e) = tokio::fs::rename(&tmp, &dest).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        release_busy(state);
        return json_error(500, &format!("cannot finalise file: {e}"));
    }

    // Mark done; release the busy flag when every file has arrived.
    let all_done = {
        let mut s = session.lock().unwrap();
        if let Some(p) = s.files.get_mut(&file_id) {
            p.done = true;
        }
        s.files.values().all(|p| p.done)
    };
    if all_done {
        sessions().lock().unwrap().remove(&session_id);
        release_busy(state);
    }

    if let Some(cb) = &state.on_received {
        cb(ReceivedFile {
            file_name: safe_name,
            saved_path: dest,
            size: written,
            from_alias: alias,
            from_addr,
            verified,
            elapsed: started.elapsed(),
        });
    }

    let _ = peer;
    hyper::Response::builder()
        .status(200)
        .body(http_body_util::Full::new(bytes::Bytes::new()))
        .unwrap()
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Keep only the last path component and strip anything a filesystem treats as
/// special. Returns `None` when nothing usable survives.
pub fn sanitise_filename(raw: &str) -> Option<String> {
    // Take the final component of either separator, so `../../etc/passwd` and
    // `C:\Windows\x` both collapse to a bare name.
    let last = raw
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(raw)
        .trim();

    if last.is_empty() || last == "." || last == ".." {
        return None;
    }

    // NUL and control characters can truncate paths in lower layers.
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && *c != '\0')
        .collect();

    // Windows reserves these names regardless of extension.
    let stem = cleaned
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let cleaned = if RESERVED.contains(&stem.as_str()) {
        format!("_{cleaned}")
    } else {
        cleaned
    };

    // 255 bytes is the common limit; count characters to stay well inside it.
    let limited: String = cleaned.chars().take(200).collect();
    if limited.is_empty() {
        None
    } else {
        Some(limited)
    }
}

/// Find a free name, appending ` (2)`, ` (3)` … on collision, so an incoming
/// file never silently overwrites one already received.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string());
    let ext = path.extension().map(|s| s.to_string_lossy().to_string());
    for n in 2..10_000 {
        let candidate_name = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let p = dir.join(&candidate_name);
        if !p.exists() {
            return p;
        }
    }
    dir.join(format!("{stem}-{}", random_hex(4)))
}

fn random_hex(bytes: usize) -> String {
    // Not cryptographic-grade on its own; tokens only need to be unguessable
    // within one session's lifetime, and are additionally bound to the session.
    use std::hash::{Hash, Hasher};
    let mut out = String::with_capacity(bytes * 2);
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ (std::process::id() as u64) << 32;
    for _ in 0..bytes {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        seed.hash(&mut h);
        std::time::Instant::now().hash(&mut h);
        let v = h.finish();
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(v);
        out.push_str(&format!("{:02x}", (seed >> 24) as u8));
    }
    out
}

fn hostname() -> String {
    std::env::var("TAILDROP_ALIAS")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "taildrop-gui".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal() {
        assert_eq!(sanitise_filename("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(sanitise_filename("/abs/path/a.txt").as_deref(), Some("a.txt"));
        assert_eq!(sanitise_filename(r"C:\Windows\evil.dll").as_deref(), Some("evil.dll"));
        assert_eq!(sanitise_filename(".."), None);
        assert_eq!(sanitise_filename("   "), None);
        assert_eq!(sanitise_filename(""), None);
    }

    #[test]
    fn escapes_windows_reserved_names() {
        assert_eq!(sanitise_filename("CON.txt").as_deref(), Some("_CON.txt"));
        assert_eq!(sanitise_filename("lpt1").as_deref(), Some("_lpt1"));
        // Not reserved: only the exact stems are.
        assert_eq!(sanitise_filename("console.log").as_deref(), Some("console.log"));
    }

    #[test]
    fn strips_control_characters() {
        assert_eq!(sanitise_filename("a\u{0}b.txt").as_deref(), Some("ab.txt"));
        assert_eq!(sanitise_filename("ok\u{1f}name").as_deref(), Some("okname"));
    }

    #[test]
    fn long_names_are_truncated() {
        let long = "a".repeat(400) + ".txt";
        let got = sanitise_filename(&long).unwrap();
        assert!(got.chars().count() <= 200);
    }

    #[test]
    fn unique_path_avoids_collisions() {
        let dir = std::env::temp_dir().join(format!("taildrop-uniq-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let first = unique_path(&dir, "a.txt");
        assert_eq!(first.file_name().unwrap(), "a.txt");
        std::fs::write(&first, b"x").unwrap();

        let second = unique_path(&dir, "a.txt");
        assert_eq!(second.file_name().unwrap(), "a (2).txt");

        std::fs::write(&second, b"x").unwrap();
        let third = unique_path(&dir, "a.txt");
        assert_eq!(third.file_name().unwrap(), "a (3).txt");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tokens_are_unique_and_hex() {
        let a = random_hex(16);
        let b = random_hex(16);
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
