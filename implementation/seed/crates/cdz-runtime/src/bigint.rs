//! Arbitrary-precision signed integers for the runtime — provided by the shared `etude-bigint` crate
//! (operator directive seq-1332: migrate cdz-runtime's value data structures onto the etude crates).
//!
//! This module is a thin re-export of [`etude_bigint::Big`]; the runtime's `BigInt` heap leaf and every
//! `bigint-*` op (`scalars.rs`) builds and reads a `Big` through it. The canonical heap-leaf byte form —
//! `[sign][little-endian magnitude, trailing-zeros-stripped]` — is IDENTICAL to the former in-tree limb
//! library: the limb width (etude stores base-2⁶⁴ limbs, the former stored base-2³²) is invisible in the
//! trailing-zero-stripped little-endian byte stream, so every serialized `Int` leaf and the
//! `champ_hash`/`champ_eq`/`value-eq` a `BigInt` map key / `=` compare relies on are byte-for-byte
//! unchanged. Only the runtime wasm's arithmetic CODE differs (u64 vs u32 schoolbook), so
//! `REQUIRED_RUNTIME_HASH` is re-frozen once as a managed consequence of the swap. The differential
//! oracle against `num-bigint` (the correctness safety net) now lives in the `etude-bigint` crate itself.

pub use etude_bigint::Big;
