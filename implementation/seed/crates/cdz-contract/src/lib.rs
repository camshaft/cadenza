//! Content hashing (`design/cadenza-platform.md` §8) and the contract-id computation (§1).
//!
//! The two things the design says "reduce to hashing", in one dep-minimal crate the platform builds on and
//! any other program can turn into a wasm component:
//!
//! - [`Hash`] — the self-describing content hash that is the system's sole identity (§8): a [`HashTag`] byte
//!   plus a blake3 digest, rendered as [`base62`] text. A blob is addressed by the hash of its bytes; an
//!   id is the hash of what it names.
//! - [`contract_id`] / [`contract_declaration`] — a contract's identity (§1): its declaration
//!   `(contract <name> (types…) <input> <output>)` built with the canonical Cadenza AST, canonicalized and
//!   encoded, and hashed. The id is a pure function of the declaration, so every producer of the same
//!   declaration — this crate, the platform's `Contract`, a nix step over a directory of contracts — agrees
//!   by construction.
//!
//! `cdz-platform` depends on this crate and re-exports [`Hash`]/[`HashTag`]/[`Hasher`]/[`base62`], and its
//! `Contract` derives its id through [`contract_declaration`] — so the hashing and the declaration encoding
//! live here once, with no second copy.

mod contract;
mod hash;

pub use contract::{
    contract_declaration, contract_id, id_name_from_descriptor, identity_from_descriptor,
};
pub use hash::{Hash, HashTag, Hasher, base62};
