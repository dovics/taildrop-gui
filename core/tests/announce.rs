//! Announcements are our own addition to LocalSend's wire format, so the one
//! thing that must hold is that a Taildrop receiver really does see them and a
//! peer that is not Taildrop really does fail (which the caller then tolerates).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use taildrop_core::localsend::identity::Identity;
use taildrop_core::localsend::receiver::{self, Announcement, ServerConfig};
use taildrop_core::localsend::sender::{self, Endpoint};
use taildrop_core::localsend::types::{DeviceInfo, DeviceType, Protocol};

fn info(identity: &Identity, alias: &str, port: u16) -> DeviceInfo {
    DeviceInfo {
        alias: alias.to_string(),
        version: "2.2".to_string(),
        device_model: None,
        device_type: Some(DeviceType::Headless),
        fingerprint: identity.fingerprint.clone(),
        port,
        protocol: Protocol::Https,
        download: false,
    }
}

#[tokio::test]
async fn announce_reaches_a_taildrop_receiver() {
    taildrop_core::init_crypto();

    let listener_identity = Identity::generate("listener").unwrap();
    let seen: Arc<Mutex<Vec<Announcement>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();

    let inbox = std::env::temp_dir().join(format!("taildrop-announce-{}", std::process::id()));
    let handle = receiver::start(ServerConfig {
        identity: listener_identity,
        inbox: inbox.clone(),
        port: 0,
        pin: None,
        policy: None,
        on_received: None,
        on_announce: Some(Arc::new(move |a| sink.lock().unwrap().push(a))),
    })
    .await
    .unwrap();

    let sender_identity = Identity::generate("caller").unwrap();
    let client = sender::build_client(&sender_identity).unwrap();
    let endpoint = Endpoint::new("127.0.0.1", handle.local_addr.port(), Protocol::Https);

    sender::announce(
        &client,
        &endpoint,
        &info(&sender_identity, "caller", 53318),
        Duration::from_secs(3),
    )
    .await
    .expect("a Taildrop receiver accepts an announcement");

    let got = seen.lock().unwrap();
    assert_eq!(got.len(), 1, "exactly one announcement should arrive");
    assert_eq!(got[0].alias, "caller");
    assert_eq!(got[0].fingerprint, sender_identity.fingerprint);
    // The address comes from the socket, not from the body, so it cannot be
    // forged into something else by the sender.
    assert!(got[0].from_addr.starts_with("127.0.0.1:"));
    drop(got);

    handle.shutdown();
    let _ = std::fs::remove_dir_all(&inbox);
}

#[tokio::test]
async fn announce_to_a_non_taildrop_peer_is_an_error() {
    taildrop_core::init_crypto();

    // A plain TCP socket that never speaks TLS: the closest stand-in for a peer
    // running something else. The caller treats this as "not a Taildrop", and
    // the periodic scan is what covers those machines.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        // Accept and hold, so the client fails on the handshake rather than on
        // a refused connection.
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
    });

    let identity = Identity::generate("caller").unwrap();
    let client = sender::build_client(&identity).unwrap();
    let endpoint = Endpoint::new("127.0.0.1", port, Protocol::Https);

    let result = sender::announce(
        &client,
        &endpoint,
        &info(&identity, "caller", 53318),
        Duration::from_millis(500),
    )
    .await;
    assert!(
        result.is_err(),
        "a non-Taildrop peer must not look announced"
    );
}
