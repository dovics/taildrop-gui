//! LocalSend v2.2 protocol: types, identity, sender, receiver.

pub mod identity;
pub mod receiver;
pub mod sender;
pub mod types;

pub use identity::{fingerprint_from_der, Identity};
pub use types::*;

/// Protocol version we advertise. Receivers use the major part for
/// compatibility checks, so this must stay in step with the wire format.
pub const PROTOCOL_VERSION: &str = "2.2";

/// Taildrop's own listen port.
///
/// Deliberately *not* 53317: that is LocalSend's default, and two servers
/// claiming one port on a machine that runs both is a needless conflict. Our
/// discovery probes both ports, so peers running either are still found.
pub const DEFAULT_PORT: u16 = 53318;
