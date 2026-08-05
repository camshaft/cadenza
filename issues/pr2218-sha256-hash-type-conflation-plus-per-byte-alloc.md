# PR #2218 review — cdz-kernel/src/component_store.rs (v-agent-harness) — OPEN — 1 type-safety (LOW-MED) + 1 efficiency (LOW) [VERIFIED] (the SHA-256 fix for MY HIGH #2210)

https://github.com/camshaft/cadenza/pull/2218 (component_store content-verifies with SHA-256 to match the
external store; operator ruling A, the fix-forward for MY HIGH #2210). Copilot 2 inline. The SHA-256 fix
itself is correct + well-documented; these are refinements.

## `get_by_hash` verifies bytes against a SHA-256 hex derived from the `Hash` argument, but `Hash` is documented as a BLAKE3 digest → a caller passing `Hash::of(...)` (blake3) gets a mismatch that looks like corruption; introduce a distinct external-address type (or at minimum doc it here) (Copilot, component_store.rs:95) — type-safety [VERIFIED, LOW-MED]
> `get_by_hash` now verifies bytes against the SHA-256 hex string derived from the provided `Hash`, but
> the API type (`Hash`) is documented elsewhere as a blake3 digest. Without an explicit note here, callers
> may incorrectly pass `Hash::of(...)` (blake3) and get a runtime mismatch that looks like data
> corruption. Consider introducing a distinct type for external store addresses; at minimum, document that
> the `Hash` argument is expected to contain the SHA-256 digest bytes for the external store.

VERIFIED: the #2218 fix correctly content-verifies with SHA-256 (`sha256_content_address(bytes) == <hash>`,
diff:186) to match the external store, keeping blake3 for kernel-internal (diff:177-180 is a thorough
module-doc explanation). BUT the API still THREADS the SHA-256 address through the `crate::hash::Hash` type
— which is defined as blake3 (`Hash::of` = blake3, hash.rs). So `get_by_hash(&self, hash: &Hash)` now
expects `hash` to CARRY sha256 digest bytes, while the type name + its `Hash::of` constructor mean blake3.
A caller who does `store.get_by_hash(&Hash::of(bytes))` (the natural, blake3 thing) gets a
ContentAddressMismatch that looks like corruption, not a type error. LOW-MED/type-safety — the module doc
(diff:177-180) explains it well, but the TYPE still conflates two incompatible addressing schemes at the
API boundary. Fix per Copilot: introduce a distinct newtype for external-store SHA-256 addresses (e.g.
`StoreAddr([u8;32])`) so blake3 `Hash` can't be passed by mistake; at minimum, a `# Safety`/`# Note` on
`get_by_hash` stating the arg must be the SHA-256 digest. (This is the type-level residual of the concierge
ruling A — SHA-256-verify was the right call, but reusing the blake3 `Hash` type invites the exact mismatch
the fix exists to prevent.)

## `sha256_content_address` does `format!("{b:02x}")` per byte → 32 temp `String` allocs per call, on every fetch/verify (Copilot, component_store.rs:135) — efficiency [VERIFIED, LOW]
> `sha256_content_address` currently does `format!("{b:02x}")` per byte, which allocates a temporary
> `String` 32 times per call. This is unnecessary overhead on every fetch/verification; writing directly
> into the output buffer avoids those allocations.
VERIFIED-per-Copilot: per-byte `format!("{b:02x}")` heap-allocs a String for each of the 32 digest bytes,
every fetch. LOW/efficiency (fetches aren't hot — but free). Fix: `use std::fmt::Write;` + `write!(&mut s,
"{b:02x}", …)` into a pre-`String::with_capacity(64)`, or a hex helper — no per-byte alloc. v-agent-harness
owns cdz-kernel/src. PR OPEN → both foldable. The type-conflation is the one worth doing (it's the API
footgun the SHA-256 ruling introduced).
