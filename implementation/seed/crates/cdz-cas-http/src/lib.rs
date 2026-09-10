//! `cdz-cas-http` — a content-addressed, immutable blob store served over HTTP.
//!
//! Two composable halves of one wire (see the crate's `Cargo.toml` header for the full contract):
//! - [`CasServer`] — a hyper HTTP/1 server that serves a swappable [`cdz_platform::BlobStore`] as
//!   `GET`/`HEAD`/`PUT /{hash}`, validating on write that a blob's bytes hash to its key.
//! - [`HttpBlobStore`] — the HTTP-backed `BlobStore` client, so a consumer (the HTTP gateway) fetches
//!   components by hash from a remote store exactly as from an in-memory one. It IS a `BlobStore`.
//!
//! The on-wire key is the base62 [`cdz_platform::Hash`] text (the platform's sole content identity); the
//! store keys on the digest (tag-agnostic), so a component put under its `Blob` hash is fetched equally by
//! the `Program` hash that names it.

pub mod auth;
pub mod client;
pub mod error;
pub mod server;

pub use client::HttpBlobStore;
pub use error::CasError;
pub use server::{CasServer, DEFAULT_MAX_BODY_BYTES};

#[cfg(test)]
mod tests;
