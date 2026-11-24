//! Device identity: a self-signed certificate whose SHA-256 fingerprint *is*
//! the device's name on the network.
//!
//! Why a certificate and not a random string: in HTTPS mode LocalSend runs
//! mutual TLS, and the peer's identity is the fingerprint of the client
//! certificate it presents. A random string would be unverifiable — anyone
//! could claim it. Here the claim is bound to a key the peer must hold.
//!
//! The fingerprint format matters for interoperability: uppercase hex of the
//! DER-encoded certificate, no separators (see LocalSend's
//! `fingerprint_from_cert_der`).

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// A device keypair plus its stable fingerprint.
#[derive(Clone)]
pub struct Identity {
    /// PEM-encoded certificate chain (one self-signed cert).
    pub cert_pem: String,
    /// PEM-encoded private key.
    pub key_pem: String,
    /// Uppercase hex SHA-256 of the certificate's DER bytes.
    pub fingerprint: String,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never let the private key reach a log line.
        f.debug_struct("Identity")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

impl Identity {
    /// Generate a fresh self-signed identity for `alias`.
    ///
    /// The certificate's subject is deliberately uninformative: the identity is
    /// the fingerprint, so a readable CN would only invite people to trust it.
    pub fn generate(alias: &str) -> Result<Self> {
        let mut params = rcgen::CertificateParams::new(vec!["taildrop-gui.local".to_string()])
            .context("building certificate parameters")?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, format!("taildrop-gui-{alias}"));

        let key_pair = rcgen::KeyPair::generate().context("generating keypair")?;
        let cert = params
            .self_signed(&key_pair)
            .context("self-signing certificate")?;

        let cert_der = cert.der();
        let fingerprint = fingerprint_from_der(cert_der);

        Ok(Identity {
            cert_pem: cert.pem(),
            key_pem: key_pair.serialize_pem(),
            fingerprint,
        })
    }

    /// Load a previously stored identity, generating and saving one if absent.
    ///
    /// The key file is written with 0600 on Unix. Callers on macOS/Windows
    /// should prefer an OS keychain; this on-disk form is the portable fallback.
    pub fn load_or_create(dir: &Path, alias: &str) -> Result<Self> {
        let cert_path = dir.join("identity.pem");
        let key_path = dir.join("identity.key");

        if cert_path.exists() && key_path.exists() {
            let cert_pem = std::fs::read_to_string(&cert_path)
                .with_context(|| format!("reading {}", cert_path.display()))?;
            let key_pem = std::fs::read_to_string(&key_path)
                .with_context(|| format!("reading {}", key_path.display()))?;
            let der = pem_to_der(&cert_pem)?;
            return Ok(Identity {
                fingerprint: fingerprint_from_der(&der),
                cert_pem,
                key_pem,
            });
        }

        let id = Identity::generate(alias)?;
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        std::fs::write(&cert_path, &id.cert_pem)?;
        write_private(&key_path, &id.key_pem)?;
        Ok(id)
    }

    /// Default state directory (`$XDG_STATE_HOME/taildrop` or the platform equivalent).
    pub fn default_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("TAILDROP_STATE_DIR") {
            return PathBuf::from(dir);
        }
        #[cfg(target_os = "windows")]
        {
            if let Ok(appdata) = std::env::var("APPDATA") {
                return PathBuf::from(appdata).join("taildrop");
            }
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home)
                    .join("Library/Application Support/taildrop");
            }
        }
        if let Ok(state) = std::env::var("XDG_STATE_HOME") {
            return PathBuf::from(state).join("taildrop");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".local/state/taildrop");
        }
        PathBuf::from(".taildrop")
    }
}

fn write_private(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(path)?.permissions();
        perm.set_mode(0o600);
        std::fs::set_permissions(path, perm)?;
    }
    Ok(())
}

/// Uppercase hex SHA-256 over the DER certificate — LocalSend's fingerprint form.
pub fn fingerprint_from_der(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    hex::encode_upper(digest)
}

/// Extract the DER bytes of the first certificate in a PEM bundle.
fn pem_to_der(pem: &str) -> Result<Vec<u8>> {
    let begin = "-----BEGIN CERTIFICATE-----";
    let end = "-----END CERTIFICATE-----";
    let start = pem.find(begin).context("no CERTIFICATE block in PEM")? + begin.len();
    let stop = pem.find(end).context("unterminated CERTIFICATE block")?;
    let b64: String = pem[start..stop]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    decode_base64(&b64).context("decoding certificate base64")
}

/// Small base64 decoder (standard alphabet, padded). Avoids a dependency for a
/// single call site; replaced by a real crate if this ever grows.
fn decode_base64(s: &str) -> Result<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }
    let bytes: Vec<u8> = s.bytes().filter(|b| *b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for b in bytes {
        let v = lookup[b as usize];
        if v == 255 {
            anyhow::bail!("invalid base64 character {:?}", b as char);
        }
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_and_reloads_a_stable_fingerprint() {
        let dir = std::env::temp_dir().join(format!("taildrop-id-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let a = Identity::load_or_create(&dir, "box").unwrap();
        let b = Identity::load_or_create(&dir, "box").unwrap();
        assert_eq!(a.fingerprint, b.fingerprint, "fingerprint must survive reload");
        assert_eq!(a.fingerprint.len(), 64);
        assert!(a.fingerprint.chars().all(|c| c.is_ascii_hexdigit() && !c.is_lowercase()));

        // The private key must not appear in Debug output.
        let dbg = format!("{a:?}");
        assert!(!dbg.contains("PRIVATE"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn base64_roundtrip_matches_known_vector() {
        // "hello" -> aGVsbG8=
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("aGVsbG8").unwrap(), b"hello");
    }

    #[test]
    fn pem_roundtrip_preserves_der() {
        let id = Identity::generate("x").unwrap();
        let der = pem_to_der(&id.cert_pem).unwrap();
        assert_eq!(fingerprint_from_der(&der), id.fingerprint);
    }
}
