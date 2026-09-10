//! The HTTP CAS server: a hyper HTTP/1 server that serves a swappable [`BlobStore`] over the pinned wire.
//!
//! Routes are keyed by the base62 [`Hash`] text in the path (`/{hash}`):
//! - `GET  /{hash}` — the raw blob bytes (`200`), `404` on a miss, `401` if a read credential is required
//!   and missing/rejected. Immutable content → cache-friendly headers.
//! - `HEAD /{hash}` — existence: `200`/`404` (the `BlobStore::has`), same read-auth gate.
//! - `PUT  /{hash}` — publish bytes for deploy tooling. The server VALIDATES `Hash::of(body) == {hash}`
//!   (digest match, tag ignored) and stores under it, so a blob whose bytes don't match its key is
//!   impossible by construction (`400` on mismatch). Guarded by a separate WRITE credential; `405` when
//!   writes are not enabled (no write credential configured — a read-only deployment).
//!
//! Auth is a `Authorization: Bearer {credential}` gate. The store itself is unpermissioned (the hash is
//! the capability — you cannot forge bytes for a hash), so a read credential is OPTIONAL: unset ⇒ reads are
//! open; set ⇒ an optional network perimeter (`401` on mismatch). Writes always require a credential.

use crate::auth::{bearer, ct_eq};
use bytes::Bytes;
use cdz_platform::{BlobStore, Hash, HashTag};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{CACHE_CONTROL, CONTENT_TYPE, LOCATION};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::net::TcpListener;

/// The default per-PUT body-size ceiling (16 MiB) — a component (wasm) is the typical blob, well under this;
/// a larger upload is answered `413` before it reaches the store, so a foreign upload cannot exhaust node
/// memory. Tune with [`CasServer::with_max_body_bytes`].
pub const DEFAULT_MAX_BODY_BYTES: usize = 16 << 20;

/// A cache-control value for immutable content-addressed blobs: a blob's bytes never change for its hash, so
/// a client/proxy may cache it indefinitely.
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";

/// The HTTP CAS server: a swappable [`BlobStore`] plus the optional Bearer credentials. Held behind an
/// `Arc` and served on a task per connection; the store needs NO external lock because `BlobStore` methods
/// (incl. `put`) take `&self` (interior mutability) — so concurrent requests don't serialize on the store.
pub struct CasServer {
    /// The backing store — a trait object so the backend (in-memory now, on-disk/S3 later) is swappable
    /// per the brief. `Arc<dyn BlobStore>` (not `Mutex<Box<…>>`): every method is `&self`, so requests share
    /// the store concurrently with no lock (reads never block; the backend's own interior mutability, e.g.
    /// InMemoryBlobStore's `RwLock`, guards writes).
    store: Arc<dyn BlobStore>,
    /// The read credential. `None` ⇒ reads are open (the store is unpermissioned); `Some` ⇒ `GET`/`HEAD`
    /// require `Authorization: Bearer {this}`, else `401`.
    read_credential: Option<String>,
    /// The write credential. `None` ⇒ writes are DISABLED (`PUT` ⇒ `405`, a read-only deployment); `Some`
    /// ⇒ `PUT` requires `Authorization: Bearer {this}`, else `401`.
    write_credential: Option<String>,
    /// The per-PUT body ceiling; a larger body is `413`.
    max_body_bytes: usize,
}

impl CasServer {
    /// A server over an arbitrary [`BlobStore`] backend, with open reads, writes disabled, and the default
    /// body ceiling. Add credentials with [`with_read_credential`](Self::with_read_credential) /
    /// [`with_write_credential`](Self::with_write_credential).
    #[must_use]
    pub fn new(store: Box<dyn BlobStore>) -> Self {
        Self {
            store: Arc::from(store),
            read_credential: None,
            write_credential: None,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }

    /// A server over a fresh in-memory store (the v0 default backend).
    #[must_use]
    pub fn in_memory() -> Self {
        Self::new(Box::new(cdz_platform::InMemoryBlobStore::new()))
    }

    /// Require this Bearer credential on `GET`/`HEAD` (an optional network perimeter over the unpermissioned
    /// store).
    #[must_use]
    pub fn with_read_credential(mut self, credential: String) -> Self {
        self.read_credential = Some(credential);
        self
    }

    /// Enable the `PUT` write path, requiring this Bearer credential.
    #[must_use]
    pub fn with_write_credential(mut self, credential: String) -> Self {
        self.write_credential = Some(credential);
        self
    }

    /// Set the per-PUT body-size ceiling (bytes); a larger body is answered `413`.
    #[must_use]
    pub fn with_max_body_bytes(mut self, max_body_bytes: usize) -> Self {
        self.max_body_bytes = max_body_bytes;
        self
    }

    /// Accept connections on `listener` forever, serving each on its own task. Returns only on an accept
    /// error (the caller owns the listener lifecycle).
    ///
    /// # Errors
    /// Propagates a fatal [`std::io::Error`] from `TcpListener::accept`.
    pub async fn serve(self: Arc<Self>, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, _peer) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let server = Arc::clone(&self);
            tokio::spawn(async move {
                let service = service_fn(move |req: Request<Incoming>| {
                    let server = Arc::clone(&server);
                    async move { Ok::<_, Infallible>(server.handle(req).await) }
                });
                // A per-connection serve error (client hang-up, malformed frame) is that connection's
                // business — drop it, keep accepting.
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .await;
            });
        }
    }

    /// Serve one request. `POST /` is a SERVER-ASSIGNED content-addressed write — the server hashes the body
    /// and answers `201` + `Location: /{hash}`, so a caller never has to hash. Every other method addresses a
    /// specific blob by its `{hash}` key; an unparseable key is a `404` miss (it names no valid content
    /// hash), never a `400` — a client asking for garbage just misses.
    async fn handle(&self, req: Request<Incoming>) -> Response<Full<Bytes>> {
        let (parts, body) = req.into_parts();
        let key = parts.uri.path().trim_start_matches('/');

        // POST is the server-assigned write, targeting the ROOT (`/`) — the address is computed + returned,
        // not supplied by the caller. `POST /{something}` is not a route (405).
        if parts.method == Method::POST {
            if !key.is_empty() {
                return floor(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "POST the body to / (the root); the content address is assigned and returned",
                );
            }
            return self.store_assigned(&parts.headers, body).await;
        }

        let Ok(hash) = key.parse::<Hash>() else {
            return floor(StatusCode::NOT_FOUND, "not found");
        };

        match parts.method {
            Method::GET => {
                if !self.read_authorized(&parts.headers) {
                    return floor(StatusCode::UNAUTHORIZED, "unauthorized");
                }
                match self.store.get(hash).await {
                    Ok(Some(bytes)) => Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, "application/octet-stream")
                        .header(CACHE_CONTROL, IMMUTABLE_CACHE)
                        .body(Full::new(bytes))
                        .expect("a 200 blob response with static headers is always valid"),
                    Ok(None) => floor(StatusCode::NOT_FOUND, "not found"),
                    // A backend failure (disk/network/S3) is NOT a miss — surface it as 500.
                    Err(_) => floor(StatusCode::INTERNAL_SERVER_ERROR, "blob store error"),
                }
            }
            Method::HEAD => {
                if !self.read_authorized(&parts.headers) {
                    return floor(StatusCode::UNAUTHORIZED, "unauthorized");
                }
                match self.store.has(hash).await {
                    Ok(true) => Response::builder()
                        .status(StatusCode::OK)
                        .header(CACHE_CONTROL, IMMUTABLE_CACHE)
                        .body(Full::new(Bytes::new()))
                        .expect("a static 200 HEAD response is always valid"),
                    Ok(false) => floor(StatusCode::NOT_FOUND, "not found"),
                    Err(_) => floor(StatusCode::INTERNAL_SERVER_ERROR, "blob store error"),
                }
            }
            Method::PUT => {
                // The idempotent content-addressed ASSERT: store the body AT the given key and validate it.
                if let Some(deny) = self.authorize_write(&parts.headers) {
                    return deny;
                }
                let bytes = match Self::read_body(body, self.max_body_bytes).await {
                    Ok(bytes) => bytes,
                    Err(resp) => return resp,
                };
                // Validate the content-address: the body must hash (by digest) to the key it's PUT under, so
                // a stored blob whose bytes don't match its key can never exist.
                let computed = Hash::of(HashTag::Blob, &bytes);
                if computed.digest() != hash.digest() {
                    return floor(
                        StatusCode::BAD_REQUEST,
                        "hash mismatch: body does not match the key",
                    );
                }
                match self.store.put(bytes).await {
                    // Empty body — the caller already knows the address (it's the key it PUT to).
                    Ok(_) => created(&hash, Bytes::new()),
                    // The store failed to persist — surface it (the caller must know the write didn't land).
                    Err(_) => floor(StatusCode::INTERNAL_SERVER_ERROR, "blob store error"),
                }
            }
            _ => floor(StatusCode::METHOD_NOT_ALLOWED, "method not allowed"),
        }
    }

    /// The `POST /` server-assigned write: authorize, read the (bounded) body, hash it, store it, and answer
    /// `201` + `Location: /{hash}` with the base62 hash text in the body — so the caller learns the address
    /// without ever hashing.
    async fn store_assigned(
        &self,
        headers: &hyper::HeaderMap,
        body: Incoming,
    ) -> Response<Full<Bytes>> {
        if let Some(deny) = self.authorize_write(headers) {
            return deny;
        }
        let bytes = match Self::read_body(body, self.max_body_bytes).await {
            Ok(bytes) => bytes,
            Err(resp) => return resp,
        };
        let hash = Hash::of(HashTag::Blob, &bytes);
        match self.store.put(bytes).await {
            Ok(_) => created(&hash, Bytes::from(hash.to_string())),
            Err(_) => floor(StatusCode::INTERNAL_SERVER_ERROR, "blob store error"),
        }
    }

    /// Whether a request bearing `headers` may read: open when no read credential is configured, else the
    /// `Authorization: Bearer` token must equal the configured credential.
    fn read_authorized(&self, headers: &hyper::HeaderMap) -> bool {
        match &self.read_credential {
            None => true,
            Some(expected) => {
                bearer(headers).is_some_and(|got| ct_eq(got.as_bytes(), expected.as_bytes()))
            }
        }
    }

    /// Authorize a write (`PUT`/`POST`): `None` if permitted, else the floor to return — `405` when writes
    /// are disabled (no write credential configured, a read-only deployment) or `401` on a missing/wrong
    /// Bearer token.
    fn authorize_write(&self, headers: &hyper::HeaderMap) -> Option<Response<Full<Bytes>>> {
        let Some(expected) = self.write_credential.as_deref() else {
            return Some(floor(StatusCode::METHOD_NOT_ALLOWED, "writes not enabled"));
        };
        if bearer(headers).is_some_and(|got| ct_eq(got.as_bytes(), expected.as_bytes())) {
            None
        } else {
            Some(floor(StatusCode::UNAUTHORIZED, "unauthorized"))
        }
    }

    /// Read at most `max` bytes of a request `body` (the per-write ceiling), or the floor to return — `413`
    /// over the ceiling, `400` on any other read error — so a large/unbounded upload can't exhaust memory.
    async fn read_body(body: Incoming, max: usize) -> Result<Bytes, Response<Full<Bytes>>> {
        match Limited::new(body, max).collect().await {
            Ok(collected) => Ok(collected.to_bytes()),
            Err(err)
                if err
                    .downcast_ref::<http_body_util::LengthLimitError>()
                    .is_some() =>
            {
                Err(floor(StatusCode::PAYLOAD_TOO_LARGE, "payload too large"))
            }
            Err(_) => Err(floor(StatusCode::BAD_REQUEST, "bad request body")),
        }
    }
}

/// A `201 Created` for a stored blob: `Location: /{hash}` (the canonical URL) + `body` — empty for a
/// validated `PUT` (the caller already knows the address), or the base62 hash text for a server-assigned
/// `POST` (so the caller learns where it landed without hashing).
fn created(hash: &Hash, body: Bytes) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::CREATED)
        .header(LOCATION, format!("/{hash}"))
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::new(body))
        .expect("a 201 response with a valid Location is always valid")
}

/// A plain-text FLOOR response the server synthesizes without touching the store.
fn floor(status: StatusCode, message: &'static str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from_static(message.as_bytes())))
        .expect("a static floor response is always valid")
}
