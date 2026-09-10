//! The HTTP-backed [`BlobStore`] client (`HttpBlobStore`) — the composable half of the brief: an HTTP CAS
//! *is* a `cdz_platform::BlobStore`, so the gateway (or any consumer) fetches components by hash from a
//! remote store exactly as it would from an in-memory one.
//!
//! ## Transport
//! A shared, pooled [`reqwest::Client`] (operator mandate: outbound HTTP clients use reqwest with TLS +
//! connection reuse — not a fresh connection per fetch). TLS is rustls with the AWS-LC (aws-lc-rs) crypto
//! provider, installed as the process default (reqwest 0.12's own `rustls-tls` feature would pull ring, so
//! aws-lc-rs is selected via a `-no-provider` reqwest feature + the explicit install below). `aws-lc-sys`
//! builds its bundled C via cmake, so the dedicated nix check carries `cmake` in its build inputs (aarch64
//! ships pregenerated bindings → no libclang/nasm). The `Client` is cheap to `clone` (an `Arc` inside), so
//! `HttpBlobStore` stays `Clone` and one connection pool is shared.
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
use cdz_platform::{BlobStore, BlobStoreError, Hash, HashTag};
use reqwest::{Method, StatusCode};
use std::sync::Once;

/// Install the aws-lc-rs rustls [`CryptoProvider`](rustls::crypto::CryptoProvider) as the process default,
/// once. reqwest's `-no-provider` rustls TLS resolves its provider from the process default; reqwest 0.12's
/// own `rustls-tls` feature would instead pull ring, so selecting aws-lc-rs requires this explicit install.
/// Idempotent: a prior install by another crate is fine — we ignore the `Err`.
fn ensure_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

/// An HTTP-backed content-addressed blob store. Points at a `base_url` (e.g. `http://cas.internal:8080` or
/// `https://cas.internal/blobs`, no trailing slash) and requests `{base_url}/{hash}` per operation over a
/// shared pooled [`reqwest::Client`] (TLS + keep-alive connection reuse).
#[derive(Clone, Debug)]
pub struct HttpBlobStore {
    base_url: String,
    client: reqwest::Client,
    read_credential: Option<String>,
    write_credential: Option<String>,
}

impl HttpBlobStore {
    /// A client against `base_url` (scheme + host + port [+ optional path prefix], no trailing slash), no
    /// credentials. Add them with [`with_read_credential`](Self::with_read_credential) /
    /// [`with_write_credential`](Self::with_write_credential). Builds a pooled reqwest client with the
    /// aws-lc-rs TLS provider (installed as the process default).
    ///
    /// # Panics
    /// If the reqwest client fails to build — which, with the aws-lc-rs provider installed and default
    /// settings, cannot happen in practice (a misconfiguration is a programmer error, not a runtime
    /// condition).
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        ensure_crypto_provider();
        let client = reqwest::Client::builder()
            .build()
            .expect("building a default reqwest client (aws-lc-rs provider installed) cannot fail");
        Self {
            base_url: base_url.into(),
            client,
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

    /// One request/response over the pooled client: `method {base_url}/{hash}` with the optional Bearer
    /// credential and (for `PUT`) `body`, collecting the response `(status, bytes)`. A non-empty `body` is
    /// attached (only `PUT` carries one; `GET`/`HEAD` pass `Bytes::new()`).
    async fn roundtrip(
        &self,
        method: Method,
        hash: &Hash,
        credential: Option<&str>,
        body: Bytes,
    ) -> Result<(StatusCode, Bytes), CasError> {
        let url = format!("{}/{hash}", self.base_url.trim_end_matches('/'));
        let mut request = self.client.request(method, url.as_str());
        if let Some(c) = credential {
            request = request.bearer_auth(c);
        }
        if !body.is_empty() {
            request = request.body(body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| CasError::Transport(format!("request to {url} failed: {e}")))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| CasError::Transport(format!("read response body from {url}: {e}")))?;
        Ok((status, bytes))
    }
}

/// The composable `BlobStore` face: an HTTP CAS is a drop-in `BlobStore`. Each method delegates to the raw
/// `Result`-returning method and maps [`CasError`] → [`BlobStoreError`] via `?` — so a transport/auth
/// failure PROPAGATES to the caller (no longer silently folded into a miss).
#[async_trait]
impl BlobStore for HttpBlobStore {
    async fn put(&self, bytes: Bytes) -> Result<Hash, BlobStoreError> {
        Ok(self.publish(bytes).await?)
    }

    async fn get(&self, hash: Hash) -> Result<Option<Bytes>, BlobStoreError> {
        Ok(self.fetch(hash).await?)
    }

    async fn has(&self, hash: Hash) -> Result<bool, BlobStoreError> {
        Ok(self.exists(hash).await?)
    }
}
