//! The fallible-CAS error type — surfaced by the [`HttpBlobStore`](crate::HttpBlobStore) *raw* methods
//! (`fetch`/`exists`/`publish`), which return `Result` so a caller (deploy tooling, a health probe) sees
//! WHY a fetch failed. The `BlobStore` trait itself is deterministic and does not carry a `Result` (a
//! well-formed backend absorbs transient I/O internally), so the trait impl folds these errors into
//! `None`/`false` + a `tracing::warn!` (see the client). This type is that richer channel.

use cdz_platform::Hash;
use std::fmt;

/// Why an HTTP CAS operation failed. `HashMismatch` is the security-relevant one: the server returned
/// bytes whose content hash does NOT match the requested key — the client refuses them rather than serving
/// forged content (the "hash is the capability, you cannot forge bytes for a hash" invariant, enforced on
/// the read path exactly as the gateway re-verifies).
#[derive(Debug, Clone)]
pub enum CasError {
    /// The credential was missing or rejected (`401`).
    Unauthorized,
    /// The bytes the server returned for `requested` hash to `computed` — a content-address violation.
    /// The client discards the bytes; a store can never legitimately serve bytes that don't match the key.
    HashMismatch { requested: Hash, computed: Hash },
    /// The server answered with an unexpected HTTP status (not 200/404/401 for a read, not 200/201 for a
    /// write).
    UnexpectedStatus(u16),
    /// A transport-level failure: DNS/connect/handshake/read, or a malformed `base_url`. Distinct from a
    /// `404` miss — this is "could not talk to the store", not "the store does not hold it".
    Transport(String),
}

impl fmt::Display for CasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => write!(f, "unauthorized (401): credential missing or rejected"),
            Self::HashMismatch {
                requested,
                computed,
            } => write!(
                f,
                "hash mismatch: requested {requested} but the returned bytes hash to {computed}"
            ),
            Self::UnexpectedStatus(code) => write!(f, "unexpected HTTP status {code}"),
            Self::Transport(msg) => write!(f, "transport error: {msg}"),
        }
    }
}

impl std::error::Error for CasError {}
