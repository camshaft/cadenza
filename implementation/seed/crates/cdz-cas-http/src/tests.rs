//! The hermetic in-process gate: stand up a [`CasServer`] on an ephemeral port and drive it with the
//! [`HttpBlobStore`] client over a real socket. This is the brief's required gate — PUT → GET round-trips,
//! a wrong/absent hash misses, a bad credential is `401`, HEAD reflects existence, and the write path
//! validates the content-address so a stored blob can never mismatch its key.

use crate::{CasServer, DiskBlobStore, HttpBlobStore};
use bytes::Bytes;
use cdz_platform::{BlobStore, Hash, HashTag};
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
    // Install the aws-lc-rs provider so a bare reqwest client builds even in a raw_put-only test
    // (HttpBlobStore installs it too; the call is idempotent).
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
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

#[tokio::test]
async fn serves_from_a_disk_backend_and_persists() {
    // A CasServer over the on-disk backend, driven over HTTP by the client — proves the swappable backend
    // composes behind the same wire, and that a published blob really lands on disk.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("cdz-cas-srv-disk-{}-{nanos}", std::process::id()));
    let server = Arc::new(
        CasServer::new(Box::new(DiskBlobStore::open(&dir).expect("open")))
            .with_write_credential(WRITE.to_string()),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(server.serve(listener));

    let http =
        HttpBlobStore::new(format!("http://{addr}")).with_write_credential(WRITE.to_string());
    let payload = Bytes::from_static(b"disk-backed over http");
    let hash = http.publish(payload.clone()).await.expect("publish");
    assert_eq!(
        http.fetch(hash).await.expect("fetch"),
        Some(payload.clone())
    );

    // The blob is actually on disk: a fresh DiskBlobStore on the same dir holds it.
    let reopened = DiskBlobStore::open(&dir).expect("reopen");
    assert_eq!(reopened.get(hash).await.unwrap(), Some(payload));

    std::fs::remove_dir_all(&dir).ok();
}

/// A raw POST to `path`, returning `(status, Location header, body text)`.
async fn raw_post(
    addr: SocketAddr,
    path: &str,
    credential: Option<&str>,
    body: Vec<u8>,
) -> (u16, Option<String>, String) {
    let client = reqwest::Client::new();
    let mut request = client.post(format!("http://{addr}{path}")).body(body);
    if let Some(c) = credential {
        request = request.bearer_auth(c);
    }
    let resp = request.send().await.expect("send");
    let status = resp.status().as_u16();
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let text = resp.text().await.expect("body");
    (status, location, text)
}

#[tokio::test]
async fn post_to_root_assigns_the_address_and_returns_it() {
    let addr = spawn_server().await;
    let payload = b"post me, don't make me hash".to_vec();
    let expected = Hash::of(HashTag::Blob, &payload);

    // POST to / — the server hashes the body and hands back the address; the caller never hashes.
    let (status, location, body) = raw_post(addr, "/", Some(WRITE), payload.clone()).await;
    assert_eq!(status, 201);
    let loc = format!("/{expected}");
    assert_eq!(
        location.as_deref(),
        Some(loc.as_str()),
        "Location is /{{hash}}"
    );
    // The body carries the base62 hash text, and it parses back to the same hash.
    assert_eq!(
        body.parse::<Hash>().expect("body is a base62 hash"),
        expected
    );

    // …and the blob is retrievable at the returned address.
    let store = client(addr);
    assert_eq!(
        store.fetch(expected).await.expect("fetch"),
        Some(Bytes::from(payload))
    );
}

#[tokio::test]
async fn post_to_a_non_root_path_is_405() {
    let addr = spawn_server().await;
    // POST addresses the ROOT only; a path is not a route (the address is server-assigned).
    let (status, _, _) = raw_post(addr, "/some-path", Some(WRITE), b"x".to_vec()).await;
    assert_eq!(status, 405);
}

#[tokio::test]
async fn post_without_a_valid_write_credential_is_unauthorized() {
    let addr = spawn_server().await;
    let (status, _, _) = raw_post(addr, "/", Some("wrong"), b"x".to_vec()).await;
    assert_eq!(status, 401);
}

/// Spawn a CAS server with a small per-write body ceiling (both credentials configured), for the
/// oversized-body (`413`) tests — the DoS guard that stops an unbounded upload exhausting memory.
async fn spawn_server_with_max_body(max_body_bytes: usize) -> SocketAddr {
    let server = Arc::new(
        CasServer::in_memory()
            .with_read_credential(READ.to_string())
            .with_write_credential(WRITE.to_string())
            .with_max_body_bytes(max_body_bytes),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(server.serve(listener));
    addr
}

#[tokio::test]
async fn a_post_over_the_body_ceiling_is_413() {
    let max = 8;
    let addr = spawn_server_with_max_body(max).await;

    // A body over the ceiling is refused 413 — the upload never reaches the store.
    let (over_status, over_loc, _) = raw_post(addr, "/", Some(WRITE), vec![b'x'; max + 1]).await;
    assert_eq!(over_status, 413, "an oversized POST body is 413");
    assert_eq!(over_loc, None, "a rejected upload assigns no address");

    // A body exactly at the ceiling still succeeds (the limit is inclusive — pins the off-by-one).
    let (at_status, _, _) = raw_post(addr, "/", Some(WRITE), vec![b'x'; max]).await;
    assert_eq!(at_status, 201, "a body exactly at the ceiling is accepted");
}

#[tokio::test]
async fn a_put_over_the_body_ceiling_is_413() {
    let max = 8;
    let addr = spawn_server_with_max_body(max).await;

    // The ceiling is enforced BEFORE the content-address check, so an oversized body is 413 regardless
    // of the key it is PUT under (the body is never fully read, let alone hashed).
    let over = vec![b'x'; max + 1];
    let over_key = Hash::of(HashTag::Blob, &over).to_string();
    assert_eq!(
        raw_put(addr, &over_key, Some(WRITE), over).await,
        413,
        "an oversized PUT body is 413"
    );

    // A body exactly at the ceiling, PUT under its own hash, is accepted (201).
    let at = vec![b'x'; max];
    let at_key = Hash::of(HashTag::Blob, &at).to_string();
    assert_eq!(
        raw_put(addr, &at_key, Some(WRITE), at).await,
        201,
        "a body exactly at the ceiling is accepted"
    );
}

/// A [`BlobStore`] whose every method errors — to drive the server's backend-error (`500`) paths. A
/// backend failure (disk/network/S3) is NOT a miss: the server must surface it as `500`, never a `404`.
struct FailingStore;
#[async_trait::async_trait]
impl BlobStore for FailingStore {
    async fn put(&self, _bytes: Bytes) -> Result<Hash, cdz_platform::BlobStoreError> {
        Err(cdz_platform::BlobStoreError::Io(
            "failing store".to_string(),
        ))
    }
    async fn get(&self, _hash: Hash) -> Result<Option<Bytes>, cdz_platform::BlobStoreError> {
        Err(cdz_platform::BlobStoreError::Io(
            "failing store".to_string(),
        ))
    }
    async fn has(&self, _hash: Hash) -> Result<bool, cdz_platform::BlobStoreError> {
        Err(cdz_platform::BlobStoreError::Io(
            "failing store".to_string(),
        ))
    }
}

/// Spawn a CAS server whose backend store always errors (both credentials configured), for the
/// backend-error (`500`) tests.
async fn spawn_failing_server() -> SocketAddr {
    let server = Arc::new(
        CasServer::new(Box::new(FailingStore))
            .with_read_credential(READ.to_string())
            .with_write_credential(WRITE.to_string()),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(server.serve(listener));
    addr
}

/// A raw request of `method` to `/{key}` (authorized to read), returning the response status code.
async fn raw_method_status(addr: SocketAddr, method: reqwest::Method, key: &str) -> u16 {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let client = reqwest::Client::new();
    client
        .request(method, format!("http://{addr}/{key}"))
        .bearer_auth(READ)
        .send()
        .await
        .expect("send")
        .status()
        .as_u16()
}

#[tokio::test]
async fn a_backend_error_on_get_is_500_not_a_miss() {
    let addr = spawn_failing_server().await;
    let key = Hash::of(HashTag::Blob, b"anything").to_string();
    // The store errored (it could not determine an answer) — that is NOT a miss. The client must see
    // 500 so it RETRIES, rather than a 404 that would let it treat a possibly-present blob as absent.
    assert_eq!(
        raw_method_status(addr, reqwest::Method::GET, &key).await,
        500,
        "a backend error on GET is 500, not a 404 miss"
    );
}

#[tokio::test]
async fn a_backend_error_on_head_is_500_not_a_miss() {
    let addr = spawn_failing_server().await;
    let key = Hash::of(HashTag::Blob, b"anything").to_string();
    assert_eq!(
        raw_method_status(addr, reqwest::Method::HEAD, &key).await,
        500,
        "a backend error on HEAD is 500, not a 404 miss"
    );
}

#[tokio::test]
async fn a_backend_error_on_put_is_500() {
    let addr = spawn_failing_server().await;
    // The body matches its key (the content-address check passes), so the 500 is the STORE failing to
    // persist — the caller must learn the write did not land.
    let body = b"store me".to_vec();
    let key = Hash::of(HashTag::Blob, &body).to_string();
    assert_eq!(
        raw_put(addr, &key, Some(WRITE), body).await,
        500,
        "a store failure on PUT is 500"
    );
}

#[tokio::test]
async fn a_backend_error_on_post_is_500() {
    let addr = spawn_failing_server().await;
    let (status, location, _) = raw_post(addr, "/", Some(WRITE), b"store me".to_vec()).await;
    assert_eq!(status, 500, "a store failure on POST is 500");
    assert_eq!(location, None, "a failed store assigns no address");
}
