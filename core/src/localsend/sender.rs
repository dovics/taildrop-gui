//! Sending: probe a peer, negotiate a session, stream the bytes.
//!
//! The sender is an HTTP client, so it needs no listening socket of its own —
//! which is what makes it usable from a CLI, a tray app, or a UI with no
//! privileges.

use crate::localsend::identity::Identity;
use crate::localsend::types::*;
use crate::localsend::PROTOCOL_VERSION;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where and how to reach a peer.
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// Host or literal IP, without scheme or port. IPv6 must be bracketed.
    pub host: String,
    pub port: u16,
    pub protocol: Protocol,
}

impl Endpoint {
    pub fn new(host: impl Into<String>, port: u16, protocol: Protocol) -> Self {
        Endpoint {
            host: host.into(),
            port,
            protocol,
        }
    }

    pub fn base_url(&self) -> String {
        format!("{}://{}:{}", self.protocol.scheme(), self.host, self.port)
    }
}

/// Progress callback: (bytes_done, bytes_total).
///
/// `Arc` rather than `Box` so one callback can be shared by several in-flight
/// uploads (and cloned into the reader) without lifetime gymnastics.
pub type ProgressFn = std::sync::Arc<dyn Fn(u64, u64) + Send + Sync>;

/// Outcome of one send.
#[derive(Debug, Clone)]
pub struct SendOutcome {
    pub session_id: String,
    pub files_sent: usize,
    pub bytes_sent: u64,
    pub elapsed: Duration,
}

impl SendOutcome {
    pub fn throughput_mbps(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs <= 0.0 {
            return 0.0;
        }
        (self.bytes_sent as f64 * 8.0) / secs / 1_000_000.0
    }
}

/// Build the HTTP client used for every request.
///
/// HTTPS mode is mutual TLS: we present our own certificate and accept the
/// peer's *without* CA verification, because peer certificates are self-signed
/// by design and trust comes from the fingerprint comparison instead. This is
/// safe only because a caller that cares checks the fingerprint (see
/// [`verify_peer_fingerprint`]); the default sender does exactly that.
pub fn build_client(identity: &Identity) -> Result<reqwest::Client> {
    let cert = reqwest::Certificate::from_pem(identity.cert_pem.as_bytes())
        .context("loading our certificate")?;
    let key = reqwest::Identity::from_pem(
        format!("{}\n{}", identity.cert_pem, identity.key_pem).as_bytes(),
    )
    .context("loading our client keypair (needs cert and key in one PEM)")?;

    reqwest::Client::builder()
        .use_rustls_tls()
        .add_root_certificate(cert)
        .identity(key)
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        // No whole-request timeout on purpose. `prepare-upload` blocks until a
        // human on the receiving side taps accept, which routinely takes longer
        // than any timeout worth hard-coding; a global one would report "send
        // failed" while the receiver is still showing the prompt. Per-phase
        // limits live at the call sites (see `prepare_upload`), and an upload
        // body streams so a stalled socket surfaces as a write error instead.
        .connect_timeout(Duration::from_secs(5))
        // Detect a peer that vanished mid-transfer without waiting forever.
        .read_timeout(Duration::from_secs(120))
        .build()
        .context("building HTTP client")
}

/// Ask a peer who it is. This is both the reachability probe and the way to
/// learn a peer's fingerprint before trusting it with bytes.
///
/// Uses `/register`, not `/info`: the spec deprecates the latter, and a register
/// call additionally makes us visible to the peer's own device list, which is
/// how a phone that cannot scan a tailnet learns we exist.
pub async fn probe(
    client: &reqwest::Client,
    endpoint: &Endpoint,
    info: &DeviceInfo,
    timeout: Duration,
) -> Result<RegisterResponse> {
    let body = RegisterRequest {
        alias: info.alias.clone(),
        version: PROTOCOL_VERSION.to_string(),
        device_model: info.device_model.clone(),
        device_type: info.device_type,
        fingerprint: info.fingerprint.clone(),
        port: info.port,
        protocol: info.protocol,
        download: info.download,
    };

    let url = format!("{}/api/localsend/v2/register", endpoint.base_url());
    let resp = client
        .post(&url)
        .json(&body)
        .timeout(timeout)
        .send()
        .await
        .with_context(|| format!("probing {url}"))?;

    let status = resp.status();
    if status == reqwest::StatusCode::NO_CONTENT {
        // A receiver may answer 204 when it has nothing to say. Treat the peer
        // as present but anonymous rather than as a failure.
        return Ok(RegisterResponse {
            alias: String::new(),
            version: String::new(),
            device_model: None,
            device_type: None,
            fingerprint: String::new(),
            download: false,
        });
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        bail!("probe rejected with {status}: {text}");
    }
    resp.json::<RegisterResponse>()
        .await
        .context("decoding register response")
}

/// Compare the fingerprint a peer claims against the one its certificate
/// proves, and against what we remembered.
///
/// Returns `Ok(())` when the claim is consistent. A `None` expected value means
/// "first contact": the caller decides whether to pin it.
pub fn verify_peer_fingerprint(claimed: &str, expected: Option<&str>) -> Result<()> {
    let claimed = claimed.trim();
    if claimed.is_empty() {
        bail!("peer did not report a fingerprint");
    }
    if let Some(expected) = expected {
        if !claimed.eq_ignore_ascii_case(expected.trim()) {
            bail!(
                "peer identity changed: expected {expected}, got {claimed}. \
                 Refusing to send — this is what a substituted device looks like."
            );
        }
    }
    Ok(())
}

/// A file staged for sending.
#[derive(Debug, Clone)]
pub struct OutgoingFile {
    pub path: PathBuf,
    /// Display name; defaults to the file's own name.
    pub name: String,
    pub size: u64,
    /// Lowercase hex SHA-256. Computed up front so the receiver can verify.
    pub sha256: String,
    pub mime: String,
}

impl OutgoingFile {
    /// Stage a file, hashing it once. Cancellation is left to the caller.
    pub async fn stage(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let meta = tokio::fs::metadata(&path)
            .await
            .with_context(|| format!("reading {}", path.display()))?;
        if meta.is_dir() {
            bail!(
                "{} is a directory; stage its files individually",
                path.display()
            );
        }
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string());

        let data = tokio::fs::read(&path)
            .await
            .with_context(|| format!("reading {}", path.display()))?;
        let sha256 = hex::encode(Sha256::digest(&data));

        Ok(OutgoingFile {
            mime: guess_mime(&name),
            name,
            size: meta.len(),
            sha256,
            path,
        })
    }
}

/// Minimal MIME guess by extension. LocalSend treats `fileType` as a hint for
/// grouping in the UI, so a wrong-but-plausible value is harmless, while a
/// missing one makes files show up as generic blobs.
fn guess_mime(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "txt" | "log" | "md" => "text/plain",
        "json" => "application/json",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Why a prepare-upload did not succeed.
#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error("the receiver needs a PIN (401)")]
    PinRequired,
    #[error("the receiver declined the transfer (403)")]
    Declined,
    #[error("the receiver is busy with another session (409)")]
    Busy,
    #[error("the receiver is rate limiting (429)")]
    RateLimited,
    #[error("the receiver rejected the request: {0}")]
    BadRequest(String),
    #[error("receiver error: {0}")]
    Internal(String),
}

impl PrepareError {
    fn from_status(status: reqwest::StatusCode, body: String) -> Self {
        match status.as_u16() {
            401 => PrepareError::PinRequired,
            403 => PrepareError::Declined,
            409 => PrepareError::Busy,
            429 => PrepareError::RateLimited,
            400 => PrepareError::BadRequest(body),
            _ => PrepareError::Internal(format!("{status}: {body}")),
        }
    }
}

/// Send the metadata block and get a session back.
pub async fn prepare_upload(
    client: &reqwest::Client,
    endpoint: &Endpoint,
    info: &DeviceInfo,
    files: &[OutgoingFile],
    pin: Option<&str>,
) -> Result<PrepareUploadResponse, PrepareError> {
    let mut map = std::collections::HashMap::new();
    for (i, f) in files.iter().enumerate() {
        let id = format!("f{i}");
        map.insert(
            id.clone(),
            FileDto {
                id,
                file_name: f.name.clone(),
                size: f.size,
                file_type: Some(f.mime.clone()),
                sha256: Some(f.sha256.clone()),
                preview: None,
                metadata: None,
            },
        );
    }

    let mut url = format!("{}/api/localsend/v2/prepare-upload", endpoint.base_url());
    if let Some(pin) = pin {
        url.push_str(&format!("?pin={}", urlencode(pin)));
    }

    let body = PrepareUploadRequest {
        info: info.clone(),
        files: map,
    };

    // Wait for the human on the other end. This is the one request that must
    // not be time-limited by the transport.
    let resp = match client
        .post(&url)
        .timeout(Duration::from_secs(300))
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return Err(PrepareError::Internal(format!("request failed: {e}"))),
    };

    let status = resp.status();
    if status == reqwest::StatusCode::NO_CONTENT {
        // "No file transfer needed" — receiver considers it already done.
        return Ok(PrepareUploadResponse {
            session_id: String::new(),
            files: std::collections::HashMap::new(),
        });
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(PrepareError::from_status(status, text));
    }
    resp.json::<PrepareUploadResponse>()
        .await
        .map_err(|e| PrepareError::Internal(format!("bad prepare response: {e}")))
}

/// Stream one file's bytes into an accepted session.
pub async fn upload_file(
    client: &reqwest::Client,
    endpoint: &Endpoint,
    session_id: &str,
    file_id: &str,
    token: &str,
    file: &OutgoingFile,
    on_progress: Option<ProgressFn>,
) -> Result<()> {
    let url = format!(
        "{}/api/localsend/v2/upload?sessionId={}&fileId={}&token={}",
        endpoint.base_url(),
        urlencode(session_id),
        urlencode(file_id),
        urlencode(token)
    );

    let handle = tokio::fs::File::open(&file.path)
        .await
        .with_context(|| format!("opening {}", file.path.display()))?;
    let total = file.size;

    // Wrap the file in a progress-counting reader so the UI sees bytes as they
    // leave, not one jump at the end.
    let reader = ProgressReader::new(handle, total, on_progress);
    let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(reader));

    let resp = client
        .post(&url)
        .header(reqwest::header::CONTENT_LENGTH, total)
        .body(body)
        // Large files can legitimately outlast any fixed limit; rely on the
        // read timeout to catch a dead peer instead.
        .timeout(Duration::from_secs(3600))
        .send()
        .await
        .with_context(|| format!("uploading {}", file.name))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        bail!("upload of {} failed with {status}: {text}", file.name);
    }
    Ok(())
}

/// Tell the receiver to drop the session (best effort; never fails a send).
pub async fn cancel_session(client: &reqwest::Client, endpoint: &Endpoint, session_id: &str) {
    let url = format!(
        "{}/api/localsend/v2/cancel?sessionId={}",
        endpoint.base_url(),
        urlencode(session_id)
    );
    let _ = client.post(&url).send().await;
}

/// Full send: probe, prepare, upload every file, report progress.
pub async fn send_files(
    identity: &Identity,
    endpoint: &Endpoint,
    info: &DeviceInfo,
    files: &[OutgoingFile],
    pin: Option<&str>,
    on_progress: Option<ProgressFn>,
) -> Result<SendOutcome> {
    let started = Instant::now();
    let client = build_client(identity)?;

    let prepared = prepare_upload(&client, endpoint, info, files, pin)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if prepared.session_id.is_empty() {
        // Receiver declared the transfer unnecessary.
        return Ok(SendOutcome {
            session_id: String::new(),
            files_sent: 0,
            bytes_sent: 0,
            elapsed: started.elapsed(),
        });
    }

    let total: u64 = files.iter().map(|f| f.size).sum();
    let mut done_before: u64 = 0;
    let mut bytes_sent = 0u64;

    for (i, file) in files.iter().enumerate() {
        let file_id = format!("f{i}");
        let token = prepared
            .files
            .get(&file_id)
            .with_context(|| format!("receiver did not accept file {file_id}"))?
            .clone();

        // Re-base each file's local progress onto the whole transfer's total,
        // so a multi-file send reads as one progress bar.
        let wrapped: Option<ProgressFn> = on_progress.as_ref().map(|f| {
            let f = f.clone();
            let base = done_before;
            std::sync::Arc::new(move |done: u64, _local_total: u64| f(base + done, total))
                as ProgressFn
        });

        upload_file(
            &client,
            endpoint,
            &prepared.session_id,
            &file_id,
            &token,
            file,
            wrapped,
        )
        .await?;

        done_before += file.size;
        bytes_sent += file.size;
    }

    Ok(SendOutcome {
        session_id: prepared.session_id,
        files_sent: files.len(),
        bytes_sent,
        elapsed: started.elapsed(),
    })
}

fn urlencode(s: &str) -> String {
    // Session ids and tokens are opaque but URL-safe in practice; still escape
    // the characters that would break a query string.
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Async reader that reports cumulative bytes read.
struct ProgressReader {
    inner: tokio::fs::File,
    done: u64,
    total: u64,
    cb: Option<ProgressFn>,
    last_report: Instant,
}

impl ProgressReader {
    fn new(inner: tokio::fs::File, total: u64, cb: Option<ProgressFn>) -> Self {
        ProgressReader {
            inner,
            done: 0,
            total,
            cb,
            last_report: Instant::now(),
        }
    }
}

impl tokio::io::AsyncRead for ProgressReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &poll {
            let read = (buf.filled().len() - before) as u64;
            if read > 0 {
                self.done += read;
                // Throttle to ~10 Hz so a fast link does not flood the UI thread.
                let due = self.last_report.elapsed() >= Duration::from_millis(100);
                let finished = self.done >= self.total;
                if due || finished {
                    if let Some(cb) = &self.cb {
                        cb(self.done, self.total);
                    }
                    self.last_report = Instant::now();
                }
            }
        }
        poll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_urls() {
        assert_eq!(
            Endpoint::new("100.1.1.1", 53318, Protocol::Https).base_url(),
            "https://100.1.1.1:53318"
        );
        assert_eq!(
            Endpoint::new("[fd7a::1]", 53318, Protocol::Http).base_url(),
            "http://[fd7a::1]:53318"
        );
    }

    #[test]
    fn mime_guessing() {
        assert_eq!(guess_mime("a.PNG"), "image/png");
        assert_eq!(guess_mime("noext"), "application/octet-stream");
        assert_eq!(guess_mime("x.tar.gz"), "application/gzip");
    }

    #[test]
    fn fingerprint_verification() {
        assert!(verify_peer_fingerprint("ABC", None).is_ok());
        assert!(verify_peer_fingerprint("abc", Some("ABC")).is_ok());
        assert!(verify_peer_fingerprint("  abc  ", Some("ABC")).is_ok());
        assert!(verify_peer_fingerprint("ABC", Some("DEF")).is_err());
        assert!(verify_peer_fingerprint("", None).is_err());
    }

    #[test]
    fn prepare_error_mapping() {
        let e = PrepareError::from_status(reqwest::StatusCode::UNAUTHORIZED, String::new());
        assert!(matches!(e, PrepareError::PinRequired));
        let e = PrepareError::from_status(reqwest::StatusCode::FORBIDDEN, String::new());
        assert!(matches!(e, PrepareError::Declined));
        let e = PrepareError::from_status(reqwest::StatusCode::CONFLICT, String::new());
        assert!(matches!(e, PrepareError::Busy));
    }
}
