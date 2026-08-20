//! The Cadenza platform runtime (`design/cadenza-platform.md`).
//!
//! A single unified crate, built bottom-up. This first slice is the content-hash foundation
//! (spec section 8): [`Hash`], the sole identity in the system. A contract-id is the hash of a
//! contract declaration; a blob is addressed by the hash of its bytes; the content-addressed store
//! is unpermissioned because the hash *is* the capability. Everything downstream — routing, the
//! registry, the store — addresses by it, so it is the correct first primitive to settle.

mod blob_store;
mod contract;
mod hash;
mod kv;
mod reducer;
mod registry;
mod str;

pub use blob_store::{BlobStore, InMemoryBlobStore};
pub use contract::Contract;
pub use hash::{Hash, base64url};
pub use kv::{InMemoryKvStore, KeyRange, KvKeyScan, KvScan, KvStore, prefix_range};
pub use reducer::{Error, Message, Outcome, Reducer, Request, Response};
pub use registry::HandlerRegistry;
pub use str::Str;

// Re-export the byte-buffer type the platform marshals through, so downstream code depends on the
// platform's chosen `Bytes` (spec §12: every byte buffer is `bytes::Bytes`, never `Vec<u8>`).
pub use bytes::Bytes;
