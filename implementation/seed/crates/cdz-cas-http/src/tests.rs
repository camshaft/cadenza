//! The hermetic in-process gate: stand up a [`CasServer`] on an ephemeral port and drive it with the
//! [`HttpBlobStore`] client over a real socket. This is the brief's required gate — PUT → GET round-trips,
//! a wrong/absent hash misses, a bad credential is `401`, HEAD reflects existence, and the write path
//! validates the content-address so a stored blob can never mismatch its key.

use crate::{CasServer, HttpBlobStore};
use bytes::Bytes;
use cdz_platform::{Hash, HashTag};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

const READ: &str = "read-credential";
const WRITE: &str = "write-credential";

/// Spawn a CAS server (with both credentials configured) on an ephemeral port; return its address.
async fn spawn_server() -> SocketAddr {
    let server = Arc::new(
        CasServer::in_memory()
            .with_read_credential(READ.to_string())
            .with_write_credential(WRITE.to_string()),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(server.serve(listener));
    addr
}

/// A fully-credentialed client for `addr`.
fn client(addr: SocketAddr) -> HttpBlobStore {
    HttpBlobStore::new(format!("http://{addr}"))
        .with_read_credential(READ.to_string())
        .with_write_credential(WRITE.to_string())
}

/// A raw PUT (bypassing the client's local-hash computation) so a mismatched key / missing credential can
/// be exercised directly against the server. Returns the response status code.
async fn raw_put(addr: SocketAddr, key: &str, credential: Option<&str>, body: Vec<u8>) -> u16 {
    // Install the ring provider so a bare reqwest client builds even in a raw_put-only test (HttpBlobStore
    // installs it too; the call is idempotent).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::new();
    let mut request = client.put(format!("http://{addr}/{key}")).body(body);
    if let Some(c) = credential {
        request = request.bearer_auth(c);
    }
    request.send().await.expect("send").status().as_u16()
}

#[tokio::test]
async fn put_then_get_round_trips_by_hash() {
    let addr = spawn_server().await;
    let store = client(addr);
    let payload = Bytes::from_static(b"the hash is the capability");

    // publish returns the content hash; fetching it back returns exactly the bytes.
    let hash = store.publish(payload.clone()).await.expect("publish");
    assert_eq!(hash, Hash::of(HashTag::Blob, &payload));
    assert_eq!(store.fetch(hash).await.expect("fetch"), Some(payload));

    // HEAD reflects existence.
    assert!(store.exists(hash).await.expect("exists"));

    // The store keys on the digest, so the same content is reachable by a Program-tagged hash too.
    let program = Hash::of(HashTag::Program, b"the hash is the capability");
    assert!(store.exists(program).await.expect("exists by program hash"));
}

#[tokio::test]
async fn getting_an_absent_hash_is_a_miss() {
    let addr = spawn_server().await;
    let store = client(addr);
    let absent = Hash::of(HashTag::Blob, b"never stored");
    assert_eq!(store.fetch(absent).await.expect("fetch"), None);
    assert!(!store.exists(absent).await.expect("exists"));
}

#[tokio::test]
async fn a_bad_read_credential_is_unauthorized() {
    let addr = spawn_server().await;
    // A client whose read credential is wrong is rejected on GET and HEAD.
    let bad = HttpBlobStore::new(format!("http://{addr}"))
        .with_read_credential("wrong".to_string())
        .with_write_credential(WRITE.to_string());
    // Store a blob (with a valid write credential) so the miss is not what causes the 401.
    let hash = bad
        .publish(Bytes::from_static(b"payload"))
        .await
        .expect("publish");
    assert!(matches!(
        bad.fetch(hash).await,
        Err(crate::CasError::Unauthorized)
    ));
    assert!(matches!(
        bad.exists(hash).await,
        Err(crate::CasError::Unauthorized)
    ));
}

#[tokio::test]
async fn a_bad_write_credential_is_unauthorized() {
    let addr = spawn_server().await;
    let bad =
        HttpBlobStore::new(format!("http://{addr}")).with_write_credential("wrong".to_string());
    assert!(matches!(
        bad.publish(Bytes::from_static(b"payload")).await,
        Err(crate::CasError::Unauthorized)
    ));
}

#[tokio::test]
async fn the_write_path_rejects_a_body_that_does_not_match_its_key() {
    let addr = spawn_server().await;
    // PUT bytes under a key that is a valid hash but NOT the hash of the body → 400. This is why a stored
    // blob whose bytes don't match its key is impossible by construction.
    let wrong_key = Hash::of(HashTag::Blob, b"some other content").to_string();
    let status = raw_put(addr, &wrong_key, Some(WRITE), b"actual body".to_vec()).await;
    assert_eq!(status, 400);

    // The matching key is accepted (201).
    let body = b"actual body".to_vec();
    let right_key = Hash::of(HashTag::Blob, &body).to_string();
    assert_eq!(raw_put(addr, &right_key, Some(WRITE), body).await, 201);
}

#[tokio::test]
async fn an_unparseable_key_is_a_miss_not_an_error() {
    let addr = spawn_server().await;
    // `not-a-hash` is not valid base62-of-33-bytes → the server 404s (a miss), never 400/500.
    let status = raw_put(addr, "not-a-hash", Some(WRITE), b"x".to_vec()).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn a_read_only_server_disables_writes() {
    // No write credential configured → PUT is 405 regardless of any credential presented.
    let server = Arc::new(CasServer::in_memory());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(server.serve(listener));

    let body = b"payload".to_vec();
    let key = Hash::of(HashTag::Blob, &body).to_string();
    assert_eq!(raw_put(addr, &key, Some("any"), body).await, 405);

    // Reads are open (no read credential configured).
    let store = HttpBlobStore::new(format!("http://{addr}"));
    let absent = Hash::of(HashTag::Blob, b"nope");
    assert_eq!(store.fetch(absent).await.expect("fetch"), None);
}
