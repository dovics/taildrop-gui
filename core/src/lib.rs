//! Taildrop core: tailnet-native file transfer.
//!
//! Two layers, deliberately separate:
//!   * [`tailscale`] — who is on my tailnet, how to reach them (LocalAPI only).
//!   * [`localsend`] — how to move bytes and prove identity (LocalSend v2.2).
//!
//! Keeping them apart means the transport works on plain LAN too, and the
//! discovery layer can be tested without any network at all.

pub mod localsend;
pub mod tailscale;

/// Install the process-wide rustls crypto provider.
///
/// Both reqwest and our own TLS server pull rustls, which then refuses to pick
/// a backend on its own. Any binary linking this crate must call this once,
/// before building an HTTP client or a listener; it is idempotent.
pub fn init_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
