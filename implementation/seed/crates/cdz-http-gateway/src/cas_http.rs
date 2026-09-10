//! The HTTP content-addressed store client (`DESIGN-http-outpost.md` §3, the dumb-gateway redirect) — behind
//! the `host` feature.
//!
//! In the redirect the gateway holds no local files: it fetches every program (the root router + the
//! subprograms it dispatches to) and their content-addressed dependencies BY HASH from a CAS reached over
//! HTTP, whose URL + credential the control server ships in the [`ControlConfig`](crate::codec::ControlConfig).
//! [`HttpCas`] is that client, implemented as a [`cdz_platform::BlobStore`] so it drops straight into the
//! [`WasmProgramStore`](cdz_platform::WasmProgramStore) in place of an in-memory CAS.
//!
//! Wire (agreed with `v-cas-http`): `GET|HEAD {base_url}/{hash}` where `{hash}` is the **base62** text of a
//! `cdz_platform::Hash` (its [`Display`] — the ONE textual form, §8; NOT hex — operator-mandated). Programs
//! are addressed by their `ProgramHash`'s base62, deps by their own hash's base62; the store keys on that
//! same base62 string, so tag-consistency is by convention (whoever PUT a blob used the tag the fetcher
//! GETs with). `Authorization: Bearer <utf8(credential)>` when the credential is non-empty; `200` body = the
//! raw bytes, `404` = absent, `401` = bad credential. **Content-verified:** the fetched bytes must hash (by
//! digest) to the requested hash — the "hash is the capability" invariant (§8), so a lying or misconfigured
//! store cannot substitute bytes.
//!
//! v0 is plain `http` (no TLS) — an internal-network CAS; `https` support is a later slice. READ-ONLY for
//! the gateway: it holds the store behind an `Arc<dyn BlobStore>` and only ever `get`/`has`, so [`put`]
//! (`&mut self`) is uncallable through that shared reference — writes are the CAS's own PUT path (deploy
//! tooling), not the gateway.

use async_trait::async_trait;
use bytes::Bytes;
use cdz_platform::{BlobStore, Hash, HashTag};
use http_body_util::{BodyExt, Empty};
use hyper::{Method, Request, Uri};
use hyper_util::rt::TokioIo;

/// A [`BlobStore`] backed by an HTTP content-addressed store (the control-server-supplied CAS). Read-only for
/// the gateway (see the module docs); fetch by hash, content-verified.
pub struct HttpCas {
    /// `host:port` to TCP-connect (http default port 80 if the URL omits it).
    authority: String,
    /// The URL path prefix (e.g. `/blobs`), no trailing slash; a fetch is `{path_prefix}/{hash}`.
    path_prefix: String,
    /// The bearer credential; empty = no `Authorization` header (dev / no-auth store).
    credential: Bytes,
}

impl HttpCas {
    /// A client for the CAS at `base_url` (e.g. `http://cas.host:8443/blobs`) presenting `credential`.
    /// `None` if `base_url` is not a valid `http://…` URL with a host (v0 rejects `https`/other schemes —
    /// TLS is a later slice).
    #[must_use]
    pub fn new(base_url: &str, credential: Bytes) -> Option<Self> {
        let uri: Uri = base_url.parse().ok()?;
        if uri.scheme_str() != Some("http") {
            return None;
        }
        let host = uri.host()?;
        let port = uri.port_u16().unwrap_or(80);
        Some(Self {
            authority: format!("{host}:{port}"),
            path_prefix: uri.path().trim_end_matches('/').to_string(),
            credential,
        })
    }

    /// Issue one `method` request for `{path_prefix}/{hash}` and return `(status, body)`, or `None` on a
    /// connect/handshake/transport error (treated as absence by the callers).
    async fn request(&self, method: Method, hash: Hash) -> Option<(u16, Bytes)> {
        let stream = tokio::net::TcpStream::connect(&self.authority).await.ok()?;
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
                .await
                .ok()?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        // Keyed on the hash's base62 text (its Display, §8 — operator-mandated, not hex); origin-form path
        // + a Host header.
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("{}/{hash}", self.path_prefix))
            .header(hyper::header::HOST, &self.authority);
        // A non-empty credential rides as `Authorization: Bearer <token>` (a bearer token is UTF-8; a
        // non-UTF-8 credential is treated as no-auth rather than failing the fetch).
        let token = std::str::from_utf8(&self.credential).unwrap_or("");
        if !token.is_empty() {
            builder = builder.header(hyper::header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let resp = sender
            .send_request(builder.body(Empty::<Bytes>::new()).ok()?)
            .await
            .ok()?;
        let status = resp.status().as_u16();
        let body = resp.into_body().collect().await.ok()?.to_bytes();
        Some((status, body))
    }
}

#[async_trait]
impl BlobStore for HttpCas {
    async fn put(&mut self, _bytes: Bytes) -> Hash {
        // The gateway holds `HttpCas` behind an `Arc<dyn BlobStore>` and only ever `get`/`has` — `put`
        // (`&mut self`) is uncallable through a shared reference, so this is unreachable in the gateway.
        // Writes are the CAS's own PUT path (deploy tooling / v-cas-http), never the gateway.
        unreachable!(
            "HttpCas is a read-only gateway CAS client; writes go through the CAS's PUT path"
        )
    }

    async fn get(&self, hash: Hash) -> Option<Bytes> {
        let (status, body) = self.request(Method::GET, hash).await?;
        if status != 200 {
            return None; // 404 absent / 401 bad-cred / other — a program the gateway can't load
        }
        // The bytes must content-address to the requested hash (digest, tag-agnostic) — never trust the CAS.
        (Hash::of(HashTag::Blob, &body).digest() == hash.digest()).then_some(body)
    }

    async fn has(&self, hash: Hash) -> bool {
        matches!(self.request(Method::HEAD, hash).await, Some((200, _)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::Full;
    use hyper::service::service_fn;
    use hyper::{Response, StatusCode};
    use std::sync::Arc;

    /// A stub CAS HTTP server: serves `blob` at `GET /{key.base62}` (200), everything else 404. Optionally
    /// serves wrong bytes instead (to exercise content-verification rejection). `key` is the hash the store
    /// keys the blob under (its base62 Display), letting a test store under one tag and fetch under another.
    async fn spawn_stub_cas_keyed(
        key: Hash,
        blob: Bytes,
        serve_wrong: bool,
    ) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // The store keys on the hash's base62 text (its Display) — the same key the client requests.
        let hash_text = key.to_string();
        let served: Bytes = if serve_wrong {
            Bytes::from_static(b"these are not the bytes you asked for")
        } else {
            blob
        };
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let hash_text = hash_text.clone();
                let served = served.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |req: Request<hyper::body::Incoming>| {
                        let hash_text = hash_text.clone();
                        let served = served.clone();
                        async move {
                            let hit = req.uri().path() == format!("/{hash_text}");
                            let resp = if hit {
                                Response::new(Full::new(served))
                            } else {
                                Response::builder()
                                    .status(StatusCode::NOT_FOUND)
                                    .body(Full::new(Bytes::new()))
                                    .unwrap()
                            };
                            Ok::<_, std::convert::Infallible>(resp)
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        addr
    }

    /// Store a blob keyed under its own `Blob` hash (the common case).
    async fn spawn_stub_cas(blob: Bytes, serve_wrong: bool) -> std::net::SocketAddr {
        let key = Hash::of(HashTag::Blob, &blob);
        spawn_stub_cas_keyed(key, blob, serve_wrong).await
    }

    #[tokio::test]
    async fn fetches_and_content_verifies_a_blob_by_hash() {
        let blob = Bytes::from_static(b"a content-addressed program component");
        let hash = Hash::of(HashTag::Blob, &blob);
        let addr = spawn_stub_cas(blob.clone(), false).await;

        let cas = HttpCas::new(&format!("http://{addr}"), Bytes::new()).expect("valid url");
        assert_eq!(
            cas.get(hash).await,
            Some(blob),
            "the CAS blob fetches by hash"
        );
        assert!(cas.has(hash).await, "HEAD reports presence");

        // A different (absent) hash → None / not present.
        let absent = Hash::of(HashTag::Blob, b"never stored here");
        assert_eq!(cas.get(absent).await, None);
        assert!(!cas.has(absent).await);
    }

    #[tokio::test]
    async fn fetches_a_program_by_its_program_hash() {
        // The gateway fetches programs by their Program-tagged hash; the store keys on that hash's base62
        // (tag-consistent by convention). Content is then verified by digest.
        let blob = Bytes::from_static(b"the root router program");
        let program = Hash::of(HashTag::Program, &blob);
        let addr = spawn_stub_cas_keyed(program, blob.clone(), false).await;
        let cas = HttpCas::new(&format!("http://{addr}"), Bytes::new()).expect("valid url");
        assert_eq!(
            cas.get(program).await,
            Some(blob),
            "a program fetched by its Program hash resolves + content-verifies"
        );
    }

    #[tokio::test]
    async fn rejects_bytes_that_do_not_match_the_hash() {
        // A lying/misconfigured CAS returns 200 with the WRONG bytes → get must reject (None), never trust it.
        let blob = Bytes::from_static(b"the real bytes");
        let hash = Hash::of(HashTag::Blob, &blob);
        let addr = spawn_stub_cas(blob, true).await; // serves wrong bytes at the right path
        let cas = HttpCas::new(&format!("http://{addr}"), Bytes::new()).expect("valid url");
        assert_eq!(
            cas.get(hash).await,
            None,
            "content that does not hash to the requested hash is rejected"
        );
    }

    #[tokio::test]
    async fn a_non_http_or_malformed_url_is_rejected() {
        assert!(HttpCas::new("https://cas.host/blobs", Bytes::new()).is_none()); // TLS not v0
        assert!(HttpCas::new("not a url", Bytes::new()).is_none());
        assert!(HttpCas::new("ftp://x/y", Bytes::new()).is_none());
    }

    #[tokio::test]
    async fn a_dial_to_a_dead_cas_is_absence() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener); // nothing listening
        let cas = HttpCas::new(&format!("http://{addr}"), Bytes::new()).expect("valid url");
        assert_eq!(cas.get(Hash::of(HashTag::Blob, b"x")).await, None);
        assert!(!cas.has(Hash::of(HashTag::Blob, b"x")).await);
    }

    /// The client drops into a `WasmProgramStore` in place of an in-memory CAS: it is `dyn BlobStore`.
    #[tokio::test]
    async fn is_a_dyn_blob_store() {
        let cas: Arc<dyn BlobStore> =
            Arc::new(HttpCas::new("http://localhost:1", Bytes::new()).unwrap());
        // A get against the (unreachable) store is absence, not a panic — confirms the trait object works.
        assert_eq!(cas.get(Hash::of(HashTag::Blob, b"x")).await, None);
    }
}
