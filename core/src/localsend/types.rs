//! LocalSend v2.2 wire types, shared by the sender and the receiver.
//!
//! Field names and semantics are taken verbatim from the protocol spec so that
//! Taildrop interoperates with unmodified LocalSend clients (including the phone
//! apps): anything extra we want must live outside these structs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Device category. Only used for picking an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceType {
    Mobile,
    Desktop,
    Web,
    Headless,
    Server,
}

impl Default for DeviceType {
    fn default() -> Self {
        DeviceType::Desktop
    }
}

/// `http` (no TLS) or `https` (mutual TLS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Http,
    Https,
}

impl Protocol {
    pub fn scheme(&self) -> &'static str {
        match self {
            Protocol::Http => "http",
            Protocol::Https => "https",
        }
    }
}

/// The device-info block that appears in register, prepare-upload and
/// prepare-download payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub alias: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_type: Option<DeviceType>,
    /// Random string in HTTP mode, cert SHA-256 (uppercase hex) in HTTPS mode.
    pub fingerprint: String,
    pub port: u16,
    pub protocol: Protocol,
    /// Whether the reverse-download API is served. Defaults to false.
    #[serde(default)]
    pub download: bool,
}

/// One file's metadata, sent before any bytes move.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDto {
    pub id: String,
    pub file_name: String,
    /// Size in bytes.
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_type: Option<String>,
    /// Lowercase hex SHA-256. Receiver verifies it and answers 422 on mismatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<FileMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessed: Option<String>,
}

// --- discovery -------------------------------------------------------------

/// Multicast announcement (`announce: true`) or unicast reply (`announce: false`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MulticastMessage {
    pub alias: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_type: Option<DeviceType>,
    pub fingerprint: String,
    pub port: u16,
    pub protocol: Protocol,
    #[serde(default)]
    pub download: bool,
    #[serde(default = "default_true")]
    pub announce: bool,
}

fn default_true() -> bool {
    true
}

/// `POST /api/localsend/v2/register` request and its response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    pub alias: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_type: Option<DeviceType>,
    pub fingerprint: String,
    pub port: u16,
    pub protocol: Protocol,
    #[serde(default)]
    pub download: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterResponse {
    pub alias: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_type: Option<DeviceType>,
    pub fingerprint: String,
    #[serde(default)]
    pub download: bool,
}

// --- upload ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareUploadRequest {
    pub info: DeviceInfo,
    /// Keyed by the same id carried inside each [`FileDto`].
    pub files: HashMap<String, FileDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareUploadResponse {
    pub session_id: String,
    /// fileId -> per-file upload token.
    pub files: HashMap<String, String>,
}

// --- errors ----------------------------------------------------------------

/// The protocol's own error body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub message: String,
}

/// Why a prepare-upload was not fully accepted. Codes are the spec's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareRejection {
    /// 401 — a PIN is configured and missing or wrong.
    PinRequired,
    /// 403 — the user declined.
    Declined,
    /// 409 — another session owns the receiver right now.
    Busy,
    /// 429 — rate limited.
    TooManyRequests,
    /// 400 — malformed body.
    BadRequest,
    /// 500 — receiver-side failure.
    Internal,
}

impl PrepareRejection {
    pub fn status_code(&self) -> u16 {
        match self {
            PrepareRejection::PinRequired => 401,
            PrepareRejection::Declined => 403,
            PrepareRejection::Busy => 409,
            PrepareRejection::TooManyRequests => 429,
            PrepareRejection::BadRequest => 400,
            PrepareRejection::Internal => 500,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_info_roundtrips_with_protocol_names() {
        let json = r#"{"alias":"a","version":"2.2","fingerprint":"F","port":53317,
                       "protocol":"https","download":true}"#;
        let d: DeviceInfo = serde_json::from_str(json).unwrap();
        assert_eq!(d.protocol, Protocol::Https);
        assert!(d.download);
        let back = serde_json::to_string(&d).unwrap();
        assert!(back.contains("\"protocol\":\"https\""));
        // download=false must still be emitted: receivers read it as a flag.
        let d2 = DeviceInfo { download: false, ..d };
        assert!(serde_json::to_string(&d2).unwrap().contains("\"download\":false"));
    }

    #[test]
    fn multicast_defaults_announce_to_true() {
        let json = r#"{"alias":"a","version":"2.0","fingerprint":"F","port":1,"protocol":"http"}"#;
        let m: MulticastMessage = serde_json::from_str(json).unwrap();
        assert!(m.announce);
        assert!(!m.download);
    }

    #[test]
    fn rejection_codes_match_spec() {
        assert_eq!(PrepareRejection::PinRequired.status_code(), 401);
        assert_eq!(PrepareRejection::Declined.status_code(), 403);
        assert_eq!(PrepareRejection::Busy.status_code(), 409);
    }
}
