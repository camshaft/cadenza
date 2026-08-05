# PR #2210 review — cdz-kernel/src/component_store.rs (v-agent-harness) — OPEN — potential incompatibility [PLAUSIBLE-HIGH, verify which store] (+ coordinate v-nix)

https://github.com/camshaft/cadenza/pull/2210 (component_store — resolve component bytes from a
content-addressed store; §23 transitive-dep resolution half). Copilot 1 inline (2 sites). Same
store-layout seam as my #2208 c3 finding — and this one is a HASH-ALGORITHM mismatch, not just a doc nit.

## component_store docs+code use blake3 content addresses (`Hash::of` = blake3), but the store PRODUCERS elsewhere in the repo write SHA-256-named files → a blake3-keyed read/verify against a SHA-256-written store won't find (or would reject) the blobs (Copilot, component_store.rs:9 & :75) — incompatibility [VERIFIED both sides; PLAUSIBLE-HIGH pending WHICH store]
> The store layout docs claim `<hash>.wasm` filenames are keyed by a blake3 content address, but the store
> manifest/files produced elsewhere in this repo are SHA-256-addressed (xtask/src/main.rs:6278 and
> cdz-run/src/cli.rs:316). This should be updated to avoid documenting an incompatible layout.

VERIFIED both sides against source:
- CONSUMER (this PR): component_store.rs:14 doc "named by its blake3 content address"; code `get_by_hash`
  does `if Hash::of(&bytes) != *hash { … content-address MISMATCH }` (diff:82) reading `<root>/<hash>.wasm`
  (diff:67). And `crate::hash::Hash::of` is BLAKE3 — hash.rs:20 `Hash(*blake3::hash(bytes).as_bytes())`,
  hash.rs:3 "Everything durable … addressed by the blake3 [digest]".
- PRODUCERS (cited): xtask/src/main.rs ~6278 — "SHA-256 of the bytes, lowercase hex (the recorded hashing
  choice)", `Sha256::new()` … `.finalize()`; Copilot also cites cdz-run/src/cli.rs:316 as SHA-256.
So the kernel's content-address is blake3 while the xtask/cdz-run store layout is SHA-256. If
component_store is meant to read the SAME on-disk store those tools produce, then: (a) the `<hash>.wasm`
lookup key won't match (blake3 hex ≠ sha256 hex for the same bytes), and (b) even a found blob would FAIL
the `Hash::of(bytes) != hash` blake3 content-verify. That's a real fetch-time incompatibility, not a doc
nit.

CONFIDENCE: PLAUSIBLE-HIGH. Both algorithms are source-verified (blake3 consumer, sha256 producers). The
open question I CAN'T settle: is `component_store`/`CDZ_STORE` meant to read the xtask/cdz-run SHA-256
store, or the kernel's OWN blake3 blob-CAS (blob.rs, "self-verifying on disk", referenced at diff:234)? If
the latter (a distinct blake3 store), the doc is fine and Copilot's cross-reference is a false alarm; if
the former, it's a MED+ incompatibility. v-agent-harness (owns cdz-kernel) must confirm WHICH store §23
resolves against. NOTE: this ties directly to my #2208 c3 (the reducer_cadenza_b1_e2e `CDZ_STORE` doc said
DiskBlobStore vs `<hash>.wasm`) — the store-layout/addressing seam spans cdz-kernel + v-nix's CDZ_STORE
wiring, so worth reconciling blake3-vs-sha256 + the layout together with v-nix. Fix: EITHER correct the doc
if it's the kernel's blake3 store (and confirm producers for THAT store are blake3), OR if it must
interop with the xtask/cdz-run SHA-256 store, reconcile the addressing (the kernel can't blake3-verify a
sha256-addressed blob). PR OPEN → resolve before it wires to a real store.
