//! The HTTP-backed [`BlobStore`] client (`HttpBlobStore`) — the composable half of the brief: an HTTP CAS
//! *is* a `cdz_platform::BlobStore`, so the gateway (or any consumer) fetches components by hash from a
//! remote store exactly as it would from an in-memory one.
//!
//! ## Two channels, on purpose
//! The `BlobStore` trait is deterministic and carries NO `Result` — a well-formed backend "absorbs
//! transient I/O internally" and `get` returns `Option` where `None` is *genuine absence*. But an HTTP
//! client can fail in ways the trait can't express (a `401`, a dead connection, or — the important one —
//! bytes that don't hash to the requested key). So this type exposes BOTH:
//! - **raw methods** [`fetch`](Self::fetch)/[`exists`](Self::exists)/[`publish`](Self::publish) returning
//!   `Result<_, CasError>` — for deploy tooling / health probes that want the reason.
//! - **the `BlobStore` impl** on top, which folds a `CasError` into `None`/`false` + a `tracing::warn!`
//!   (the only shape the trait allows). A hash-mismatch or `401` thus reads as a *miss* to the consumer,
//!   which is the safe conservative behavior: better a miss (the gateway then `404`s) than serving bytes
//!   that don't match the key.
//!
//! ## Read-path verification
//! On a `200`, the client recomputes `Hash::of(bytes)` and refuses the bytes unless the digest matches the
//! requested hash — the same re-verification the gateway performs. You cannot forge bytes for a hash, so a
//! store that serves mismatched bytes is either buggy or hostile; either way the client discards them.

use crate::error::CasError;
use async_trait::async_trait;
use bytes::Bytes;
use cdz_platform::{BlobStore, Hash, HashTag};
use http_body_util::{BodyExt, Full};
use hyper::header::{AUTHORIZATION, HOST};
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

/// An HTTP-backed content-addressed blob store. Points at a `base_url` (e.g. `http://cas.internal:8080`,
/// no trailing slash) and dials `{base_url}/{hash}` per request. v0 speaks plain HTTP/1 (no TLS — a TLS
/// terminator / mesh sits in front in deployment); a raw hyper client connection is opened per request
/// (the CAS is a low-QPS control-plane fetch, not a hot path — a pool is a later optimization).
#[derive(Clone, Debug)]
pub struct HttpBlobStore {
    base_url: String,
    read_credential: Option<String>,
    write_credential: Option<String>,
}

impl HttpBlobStore {
    /// A client against `base_url` (scheme + host + port, no trailing slash), no credentials. Add them with
    /// [`with_read_credential`](Self::with_read_credential) / [`with_write_credential`](Self::with_write_credential).
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            read_credential: None,
            write_credential: None,
        }
    }

    /// Send `Authorization: Bearer {credential}` on reads (`GET`/`HEAD`).
    #[must_use]
    pub fn with_read_credential(mut self, credential: String) -> Self {
        self.read_credential = Some(credential);
        self
    }

    /// Send `Authorization: Bearer {credential}` on writes (`PUT`).
    #[must_use]
    pub fn with_write_credential(mut self, credential: String) -> Self {
        self.write_credential = Some(credential);
        self
    }

    /// GET the bytes for `hash`, verifying their content-address. `Ok(Some(bytes))` on a verified `200`,
    /// `Ok(None)` on a `404` miss.
    ///
    /// # Errors
    /// [`CasError::HashMismatch`] if the returned bytes don't hash to `hash`; [`CasError::Unauthorized`] on
    /// `401`; [`CasError::UnexpectedStatus`] on any other status; [`CasError::Transport`] on a dial/read
    /// failure or a malformed `base_url`.
    pub async fn fetch(&self, hash: Hash) -> Result<Option<Bytes>, CasError> {
        let (status, body) = self
            .roundtrip(
                Method::GET,
                &hash,
                self.read_credential.as_deref(),
                Bytes::new(),
            )
            .await?;
        match status {
            StatusCode::OK => {
                let computed = Hash::of(HashTag::Blob, &body);
                if computed.digest() != hash.digest() {
                    return Err(CasError::HashMismatch {
                        requested: hash,
                        computed,
                    });
                }
                Ok(Some(body))
            }
            StatusCode::NOT_FOUND => Ok(None),
            StatusCode::UNAUTHORIZED => Err(CasError::Unauthorized),
            other => Err(CasError::UnexpectedStatus(other.as_u16())),
        }
    }

    /// HEAD `hash`: `Ok(true)`/`Ok(false)` for present/absent.
    ///
    /// # Errors
    /// [`CasError::Unauthorized`] on `401`; [`CasError::UnexpectedStatus`] on any other non-`200`/`404`
    /// status; [`CasError::Transport`] on a dial/read failure.
    pub async fn exists(&self, hash: Hash) -> Result<bool, CasError> {
        let (status, _) = self
            .roundtrip(
                Method::HEAD,
                &hash,
                self.read_credential.as_deref(),
                Bytes::new(),
            )
            .await?;
        match status {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            StatusCode::UNAUTHORIZED => Err(CasError::Unauthorized),
            other => Err(CasError::UnexpectedStatus(other.as_u16())),
        }
    }

    /// PUT `bytes` and return their content hash. The hash is computed locally and the server re-validates
    /// it, so a successful publish means the bytes are stored under exactly this hash.
    ///
    /// # Errors
    /// [`CasError::Unauthorized`] on `401` (missing/rejected write credential); [`CasError::UnexpectedStatus`]
    /// on any other non-`2xx` status (e.g. `405` writes-disabled, `413` too large); [`CasError::Transport`]
    /// on a dial/read failure.
    pub async fn publish(&self, bytes: Bytes) -> Result<Hash, CasError> {
        let hash = Hash::of(HashTag::Blob, &bytes);
        let (status, _) = self
            .roundtrip(Method::PUT, &hash, self.write_credential.as_deref(), bytes)
            .await?;
        match status {
            StatusCode::OK | StatusCode::CREATED => Ok(hash),
            StatusCode::UNAUTHORIZED => Err(CasError::Unauthorized),
            other => Err(CasError::UnexpectedStatus(other.as_u16())),
        }
    }

    /// One request/response over a fresh HTTP/1 connection: dial the `base_url` authority, send `method
    /// /{hash}` with the optional Bearer credential + `body`, and collect the response `(status, bytes)`.
    async fn roundtrip(
        &self,
        method: Method,
        hash: &Hash,
        credential: Option<&str>,
        body: Bytes,
    ) -> Result<(StatusCode, Bytes), CasError> {
        let uri: hyper::Uri = self
            .base_url
            .parse()
            .map_err(|e| CasError::Transport(format!("bad base_url {:?}: {e}", self.base_url)))?;
        let authority = uri
            .authority()
            .map(|a| a.as_str().to_string())
            .ok_or_else(|| {
                CasError::Transport(format!(
                    "base_url {:?} has no host:port authority",
                    self.base_url
                ))
            })?;

        let stream = TcpStream::connect(authority.as_str())
            .await
            .map_err(|e| CasError::Transport(format!("connect {authority}: {e}")))?;
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
                .await
                .map_err(|e| CasError::Transport(format!("handshake {authority}: {e}")))?;
        // Drive the connection to completion on its own task while we await the response.
        tokio::spawn(async move {
            let _ = conn.await;
        });

        let mut builder = Request::builder()
            .method(method)
            .uri(format!("/{hash}"))
            .header(HOST, authority.as_str());
        if let Some(c) = credential {
            builder = builder.header(AUTHORIZATION, format!("Bearer {c}"));
        }
        let request = builder
            .body(Full::new(body))
            .map_err(|e| CasError::Transport(format!("build request: {e}")))?;

        let response = sender
            .send_request(request)
            .await
            .map_err(|e| CasError::Transport(format!("send request: {e}")))?;
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|e| CasError::Transport(format!("read response body: {e}")))?
            .to_bytes();
        Ok((status, bytes))
    }
}

/// The composable `BlobStore` face: an HTTP CAS is a drop-in `BlobStore`. A [`CasError`] is folded into the
/// trait's deterministic `Option`/`bool` shape (+ a `tracing` breadcrumb) — see the module docs.
#[async_trait]
impl BlobStore for HttpBlobStore {
    async fn put(&mut self, bytes: Bytes) -> Hash {
        // The content hash is a pure function of the bytes, so we always know what to return; a failed PUT
        // is logged (the trait can't surface it) and the hash is returned regardless — the deterministic
        // "put absorbs transient I/O" contract. Callers wanting the real outcome use `publish`.
        match self.publish(bytes.clone()).await {
            Ok(hash) => hash,
            Err(err) => {
                tracing::error!(error = %err, "cas-http put failed; returning content hash regardless");
                Hash::of(HashTag::Blob, &bytes)
            }
        }
    }

    async fn get(&self, hash: Hash) -> Option<Bytes> {
        match self.fetch(hash).await {
            Ok(found) => found,
            Err(err) => {
                tracing::warn!(error = %err, %hash, "cas-http get failed; treating as a miss");
                None
            }
        }
    }

    async fn has(&self, hash: Hash) -> bool {
        match self.exists(hash).await {
            Ok(present) => present,
            Err(err) => {
                tracing::warn!(error = %err, %hash, "cas-http has failed; treating as absent");
                false
            }
        }
    }
}
