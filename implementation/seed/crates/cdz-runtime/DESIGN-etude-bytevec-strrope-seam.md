# DESIGN — bytes/string data structures onto etude (ByteVec / StrRope)

Status: DESIGN (plan step 1). No code landed yet. Companion to the bigint→`etude-bigint`
migration already shipped (`src/bigint.rs` is now `pub use etude_bigint::Big`).

Operator directive seq-1332 ("switch all data structures to etude") + ruling seq-1382:
**build** bytevec + strrope support (not keep-in-tree). North star: **zero-cost host binary
passing** — crossing the component boundary with a bytes/string value is a refcount bump, not
a copy. The operator resolved the Perceus-coherence question directly: "leverage reference
counting for bytes and strings … the Perceus Handle would work the same way and just refer to
the bytes."

## Where this touches the runtime

Today (`src/lib.rs`, `src/bytes_string.rs`):

- A heap node is `struct Node { rc: u32, handles: Handles, raw: Raw, /*debug: guard, node_id*/ }`.
  There is no kind field — the Cadenza type is compile-time knowledge.
- `Raw` is `Inline { len: u8, buf: [u8; INLINE_RAW_CAP=12] }` | `Heap(Vec<u8>)`, deref → `&[u8]`.
- A **flat** bytes/str leaf stores its bytes in `raw`. A **concat** is a ROPE node whose
  `handles` are the child leaves (bytes/str live in `handles`, not `raw`). `op_bytes_*`/`op_str_*`
  in `bytes_string.rs` build/slice/flatten these; `fill_rope_bytes`/`bytes_flatten` walk the rope.
- Lifetime is the runtime's OWN non-atomic Perceus RC: `dup` = `rc++`, `drop` = `rc--` then free
  (the free cascade drops `handles` and the `raw` Vec). The debug live-objects census counts nodes;
  balanced-0 at program end is the leak gate.

## Target representation

A bytes/str VALUE node **holds an `etude_bytevec::ByteVec`** (a string node holds an
`etude_strrope::StrRope`, which is `ByteVec` + a UTF-8 invariant). `ByteVec` is a tiered rope over
`bytes::Bytes` chunks. Concretely a new `Raw` arm (working name `Raw::Rope(ByteVec)`), or a
dedicated node field — chosen at slice-1 implementation time to minimize the frozen-hash delta.

The runtime rope-of-nodes representation for concat goes away: concat becomes
`ByteVec::push_back`, slice becomes `ByteVec::slice` (both structural, O(log n)). `handles` for a
bytes/str node becomes empty; the byte content lives entirely inside the `ByteVec`.

## Perceus coherence — the Handle owns the value, the census counts the Handle

This is the load-bearing invariant and the operator's explicit design:

- The **Handle still owns the value.** `Node.rc` is unchanged. `dup` (rc++) shares the node and
  its `ByteVec`; `drop` at rc==0 frees the node, which **drops the `ByteVec`** (one RC decrement on
  its shared `bytes::Bytes` buffers).
- The live-objects **census counts the Handle/node**, exactly as today — NOT the internal
  `bytes::Bytes` allocations. So balanced-0 and the leak gate are unchanged in meaning: one node
  born, one node freed.
- `bytes::Bytes`'s own (atomic) refcount is **the zero-copy sharing mechanism**, not a second
  ownership discipline the census must see. It is coherent, not an invisible Arc leak: a `ByteVec`
  is owned by exactly one node at a time (Perceus linearity); sharing the underlying buffer across
  a host crossing is a controlled clone whose lifetime is still bracketed by the owning node's
  drop. When the node dies, its `ByteVec` dies, and every `Bytes` RC it held decrements.

Non-atomic-vs-atomic note: the runtime is single-threaded per component instance, so `Node.rc` is
non-atomic. `bytes::Bytes` uses atomic RC internally; that cost is paid only on clone/drop of the
buffer (host crossings and structural ops), not on the hot per-node `dup`/`drop`, so the
single-threaded fast path is preserved.

## ByteVec / StrRope API mapping (verified against the etude source)

| runtime op / need                         | etude ByteVec / StrRope                          |
|-------------------------------------------|--------------------------------------------------|
| `dup` (share)                             | `Clone` — O(1) RC bump                            |
| `drop` / free                             | `Drop`                                            |
| construct a leaf from bytes               | `From<Vec<u8>>` / `From<&[u8]>` / `From<Bytes>`   |
| `op_bytes_slice` / `op_str_slice`         | `slice(range) -> Self` — O(log n) structural share|
| concat (rope build)                       | `push_back(Bytes)` / append                       |
| byte view for `champ_hash`/`champ_eq`/cmp | `as_contiguous() -> Option<&[u8]>`, else chunk-iter (`bytes::Buf`) |
| logical equality                          | `PartialEq` (chunk-boundary-independent)          |
| host crossing (zero-copy handoff)         | clone the inner `Bytes` (RC bump)                 |

The multi-chunk case matters for the tagless byte-hash: `champ_hash`/`champ_eq`/`champ_key_cmp`
MUST produce byte-identical results to today (bytes/strings are map keys — a hash change reorders
CHAMP nodes and breaks the frozen value form). When `as_contiguous()` returns `None`, hash/compare
by iterating chunks in logical order; `PartialEq` already handles chunk-boundary-independent eq.

## Byte-form / frozen-hash discipline

Every observable byte form MUST stay identical: `value_codec` bytes/str encode/decode, the CHAMP
key hash/eq/cmp, and `print_display`/render. The migration re-freezes `REQUIRED_RUNTIME_HASH` +
`DEBUG_RUNTIME_HASH` (managed via `cargo xtask codegen`, verified NIX-canonical — native
cargo-component codegen is unfaithful for the debug-counters build; gate hash parity through v-nix,
not native codegen).

Inline/empty fast paths to preserve: the IMMORTAL empty-bytes / empty-str singletons
(`EMPTY_BYTES`, census-excluded) and the ≤`INLINE_RAW_CAP` short-leaf inline path. A `ByteVec` over
an empty/short buffer should not regress these; keep an inline or empty-singleton fast path so
short-string-heavy rope assembly does not start heap-allocating `Bytes` per tiny leaf.

## Incremental slice plan (each slice gated like the bigint migration)

Gate per slice: native `cargo test -p cdz-runtime --lib` + blast-radius corpus (bytes/string
chapters) + debug-counters live-objects==0 leak-check + v-nix NIX hash-parity, then re-freeze.

1. **This doc** (design of the seam) — committed reference. ← current.
2. **etude no_std dependency.** `etude-bytevec` is no_std-ready; `etude-strrope`/`etude-str` are
   std-configured (default `bytes` features + `std` imports). v-etude confirmed no_std strrope/str
   is FEASIBLE and is landing the feature. Must also confirm `bytes`' no_std `Bytes` builds under
   `wasm32-unknown-unknown` (atomic RC on the wasm target). Depends on v-etude; pin the rev once
   published (like the `etude-bigint` d5ef70bf pin).
3. **Migrate bytes** onto `etude-bytevec` first (a frozen-hash re-freeze slice): rewrite the `Raw`
   representation, `op_bytes_*`, the free path (drop the `ByteVec`), the `champ_*` byte-form, and
   `value_codec` bytes enc/dec. Keep the empty-singleton + inline fast paths.
4. **Migrate str** onto `etude-strrope` (a second re-freeze slice): the UTF-8 layer over ByteVec;
   `op_str_*`, str value_codec, `print_display`.
5. **Zero-copy host boundary** — the payoff. The rcdzc backend / `host.rs` marshalling passes the
   `Bytes` across the component boundary by RC bump, not a copy. This is the v-wit-boundary seam;
   coordinate ownership/consultation with `v-wit-boundary`.

## Dependencies & coordination

- **v-etude**: no_std `etude-strrope`/`etude-str` (+ `bytes` no_std under wasm32). Blocking for
  slices 4–5; slice 3 (bytevec) can begin once the no_std bytevec rev is pinnable.
- **v-wit-boundary**: owns the host-boundary marshalling seam; the zero-copy handoff (slice 5) is
  co-designed there.
- **rational** (seq-1353 "when reached") = pair-of-`Big`s reusing `etude-bigint`; a future feature
  (no in-tree `Rational` node today), out of scope for this seam.

## Open questions (resolve at slice-1 implementation)

- `Raw::Rope(ByteVec)` arm vs a dedicated node field — pick the smaller frozen-hash delta and the
  cleaner deref story (readers currently rely on `Raw: Deref<Target=[u8]>`; a multi-chunk `ByteVec`
  cannot cheaply deref to a single `&[u8]`, so byte-consumers must move to `as_contiguous()`-or-iter).
- Whether the ≤12-byte inline fast path stays as a `Raw::Inline` arm alongside the rope arm (likely
  yes — avoids a `Bytes` allocation per tiny leaf) or is folded into `ByteVec`'s own inline tier.
