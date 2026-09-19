# Design — a copy-avoiding protobuf **wire-format** tokenizer over the shared chunk-cursor

**Author:** design agent (`design-protobuf-tokenizer`).
**Audience:** the `vertical` agent that builds this (suggested `etude-protobuf`, `area=etude-protobuf`,
comms home in a cadenza worktree, mission target the standalone **etude** repo), plus `etude-json`
(shares the cursor/span/value-ref primitive), `etude-byterope-compat`/`fixer-byterope` (the byte-rope
owners), and the `concierge` (operator relay).
**Status:** DESIGN — operator is away; the scope forks below were DECIDED here with sensible defaults
per the concierge away-mode directive (note 81476: "make the calls yourself with sensible defaults,
note alternatives in the PR"). The one genuinely operator-facing fork — the shared token value
representation (chunk-refs-in-addition-to-spans) the operator raised in etude-json PR #101 — is
RESOLVED here with a recommended default (§2) and its alternatives, for async GitHub review.
**Subsystem:** a new `crates/etude-protobuf` crate in the standalone **etude** repo
(`/local/home/bythewc/Projects/camshaft/etude`, plain cargo, no nix), consuming the shared
`etude-span` cursor primitive (etude PR #101) and the `etude-bytevec` byte-rope. Mirrors the
`etude-json` vertical exactly (build → differential-oracle → benchmark → beat).

## 0. The principle — READ FIRST

The operator's spark (Slack, via concierge notes 81457 + 81476), condensed and verbatim in part:

> *"it would be nice if we could have the protobuf tokenizer returning the **chunks in addition to
> the spans**? Via some ZST or some mechanism. My worry is we have to do **logn access** for all
> these spans, right?"*

Two things are being asked for, and this design must deliver both:

1. **A schema-agnostic protobuf *wire-format* tokenizer** — the same copy-avoiding, 0-allocation,
   chunk-cursor playbook `etude-json` proved (its vertical log: tokenize allocates **0 bytes on every
   shape**, tokens are rope spans; now faster than `serde_json` on 5/6 shapes). protobuf is the second
   format to ride the shared cursor primitive being extracted into `etude-span` (etude PR #101).
2. **Tokens must carry a CHUNK reference in addition to the span**, so reading a token's bytes is
   **O(1)**, not the **O(log₃₂ n)** tree descent a bare `Span{start,end}` costs when you later
   re-read it via `ByteVec::byte_at`/`slice`. This is the crux the operator raised in PR #101 and
   explicitly deferred to this design (etude-json log: *"coordinate with design-protobuf-tokenizer …
   the next design conversation for the shared primitive"*). §2 is the heart of this doc.

**Why O(log n) is a real cost, grounded in the code.** `ByteVec` is a tiered RRB byte-rope
(`etude-bytevec/src/lib.rs`). `byte_at(offset)` and `slice(range)` walk the tree's cumulative
size-tables — **O(log₃₂ n)** per call (`lib.rs:430`, `:789`; the deep tier shares subtrees, no copy,
but still descends). A tokenizer emits *many* tokens; if every consumer of every token re-descends
the tree to read its bytes, the parse becomes O(tokens · log n) on the *read* side even though the
*scan* is O(n). The fix is to hand the consumer the contiguous leaf slice the cursor **already holds**
at emit time — for free, no second descent.

**Why it's free.** The shared scan cursor (prior art `etude-json/src/lib.rs:244`, being extracted to
`etude-span`) already exposes `chunk_tail() -> &'a [u8]` which borrows **the input, lifetime `'a`, not
`self`** (`:280`). When the tokenizer sits on a value it can capture that leaf-tail slice at zero cost.
For a value that fits inside one leaf — the overwhelmingly common case for protobuf scalars and most
LEN payloads — that slice **is** the value bytes: O(1), zero descent. That is the whole mechanism.

## 1. protobuf wire format — the scope (DECIDED: wire-only, schema-agnostic)

**DECIDED default (concierge away-mode note 81476):** a **proto3-compatible *wire-format* tokenizer**,
**NOT** a schema-driven message decoder. It tokenizes the self-describing wire structure only. It does
NOT read `.proto` files, does NOT know field names or declared types, and does NOT decide whether a
`LEN` payload is a `string`, `bytes`, a nested message, or a packed repeated field — that requires a
schema and is a strictly higher layer (out of scope; the consumer drives it, see §3.3).

The wire grammar it tokenizes (proto3 encoding spec):

- A message is a flat sequence of **records**. Each record is a **tag** (a base-128 varint) followed by
  a payload whose shape is fixed by the tag's low 3 bits (the *wire type*):

  | wire type | name    | payload                                   | proto declared types                       |
  |-----------|---------|-------------------------------------------|---------------------------------------------|
  | 0         | `Varint`| a LEB128 varint (≤10 bytes)               | int32/64, uint32/64, sint32/64, bool, enum  |
  | 1         | `I64`   | 8 bytes little-endian                     | fixed64, sfixed64, double                   |
  | 2         | `Len`   | a varint length prefix, then that many bytes | string, bytes, embedded message, packed repeated |
  | 3         | `SGroup`| (nothing — start-group marker)            | groups (proto2 legacy)                      |
  | 4         | `EGroup`| (nothing — end-group marker)              | groups (proto2 legacy)                      |
  | 5         | `I32`   | 4 bytes little-endian                     | fixed32, sfixed32, float                    |

- `tag = (field_number << 3) | wire_type`. `field_number` is a `u32` (1 ..= 2^29−1); the tag itself is
  a varint (a large field number tag can be up to 5 bytes).
- **Varint (LEB128):** 7 payload bits per byte, high bit = continuation. Max 10 bytes for a 64-bit
  value. Overlong (>10 bytes, or a 10th byte with bits set above bit 63) is malformed.
- **Zig-zag** (`sint32/64`) and **two's-complement negative** interpretation are *consumer* concerns —
  the tokenizer emits the raw decoded `u64`; the schema layer reinterprets. (Schema-agnostic: the wire
  cannot tell `int64` from `sint64`.)

**Groups (wire types 3/4)** are proto2 legacy that proto3 dropped, but they still appear on the wire.
**DECIDED default:** emit them as **structural** `GroupStart{field}` / `GroupEnd{field}` tokens (the
protobuf analog of JSON `{` `}`) and let the consumer match them by field number — a faithful wire
reader. *(Alt: reject as unsupported. Rejected as default — a wire tokenizer should read all valid
wire, and groups are valid wire.)*

## 2. THE SHARED VALUE-REFERENCE — chunks *in addition to* spans (the operator's ask)

This is the primitive the operator asked for and the one thing both tokenizers must agree on. It lands
in **`etude-span`** (the shared crate from PR #101) so `etude-json` and `etude-protobuf` share one
representation. This section RESOLVES etude-json's open PR-#101 question.

### 2.1 The value view: `Bytes<'a>` — a chunk view that embeds the span

```rust
// in etude-span
/// A view of a token's bytes, captured while the tokenizer's cursor was ON them.
/// Zero-cost to produce (the cursor already holds the leaf); O(1) to read in the
/// common single-leaf case; NEVER re-descends the rope.
#[derive(Copy, Clone)]
pub enum Bytes<'a> {
    /// The value lies entirely within ONE rope leaf: a borrowed contiguous slice.
    /// O(1) access, zero copy, zero descent — the common case for scalars and
    /// most LEN payloads. `.span` is carried alongside for portability.
    Chunk { slice: &'a [u8], span: Span },
    /// The value straddles ≥2 leaves. Carries a resumable cursor position at the
    /// value start + the length. `.chunks()` yields the covering leaf slices in
    /// O(1)-amortized (forward leaf-successor walk, NO root descent); `.span` is
    /// the absolute range for portability / random re-access.
    Split(Split<'a>),
}

impl<'a> Bytes<'a> {
    pub fn span(&self) -> Span;                 // always O(1) — the portable range
    pub fn as_slice(&self) -> Option<&'a [u8]>; // Some(_) single-leaf O(1); None if split
    pub fn chunks(&self) -> impl Iterator<Item = &'a [u8]>; // O(1)-amortized/leaf, no descent
    pub fn len(&self) -> usize;
}
```

`Split<'a>` holds the value's first leaf-tail `&'a [u8]`, a resumable `etude_bytevec::Chunks<'a>`
positioned just past it, and the remaining length + absolute `Span`. Resuming costs an O(depth)
one-time clone of the `Chunks` walk-stack (depth = log₃₂ n, a handful of pointers — **not** an
O(log n)-*per-byte* re-descent), then O(1) per subsequent leaf. This is strictly the walk the scan
already performs, replayed — never a fresh root descent.

**Key property:** `span()` always exists (portable, `Copy`, serializable, usable for diagnostics or
random access), AND the chunk view gives O(1) access. The operator gets "chunks **in addition to**
spans" — the span is not replaced, it is embedded. O(log n) is gone from the forward-read path.

### 2.2 The "ZST or some mechanism": a **capture policy** type parameter

The operator floated "via some ZST or some mechanism." The clean mechanism is a zero-sized **capture
policy** on the tokenizer, selecting what a token *stores* — monomorphized, zero runtime branch:

```rust
// in etude-span — sealed marker trait, two ZST implementors
pub trait Capture: sealed::Sealed { type Value<'a>; /* how to build from the cursor */ }
pub enum Spans {}    // ZST: Value<'a> = Span         — 16 B, Copy, portable/owned-friendly
pub enum Chunks {}   // ZST: Value<'a> = Bytes<'a>    — O(1) access (the default)

pub struct Tokenizer<'a, C: Capture = Chunks> { /* cursor + policy PhantomData */ }
```

Producing the chunk view is free at scan time (the cursor holds the leaf either way); the ZST only
decides whether it is *stored*. **DECIDED default: `Chunks`** (O(1) access — the operator's stated
want). A size-sensitive, portable, or *owned-token* consumer (e.g. persisting a token stream past the
input's lifetime) selects `Spans` and re-resolves lazily. This is the "ZST mechanism," and it costs
nothing when unused.

*(Alt considered and rejected as default: always store only `Bytes<'a>` with no policy. Rejected
because a consumer that wants the smallest possible `Copy` token, or a token outliving the borrow,
genuinely benefits from `Spans`; the ZST gives both at zero cost. Alt (c) from PR #101 — keep `Span`
only + an O(1) `resolve_at(cursor)` — is subsumed: it is exactly the `Spans` policy plus in-order
resolution, so we keep it as the opt-down path rather than the default.)*

## 3. The tokenizer shape

### 3.1 Tokens

```rust
pub struct Field<'a, C: Capture = Chunks> {
    pub number: u32,          // field_number = tag >> 3
    pub wire_type: WireType,
    pub value: Value<'a, C>,
}
pub enum WireType { Varint, I64, Len, SGroup, EGroup, I32 }

pub enum Value<'a, C: Capture> {
    Varint(u64),              // LEB128, already decoded (needed to advance anyway)
    I64(C::Value<'a>),        // 8 fixed bytes LE; helpers .as_u64() / .as_f64()
    I32(C::Value<'a>),        // 4 fixed bytes LE; helpers .as_u32() / .as_f32()
    Len(C::Value<'a>),        // length-delimited payload: string/bytes/message/packed
    GroupStart,               // wire type 3 (structural; number carried on Field)
    GroupEnd,                 // wire type 4
}
```

- **`Varint` is eagerly decoded to `u64`** — DECIDED default. You *must* decode it to know how far to
  advance and to read the tag, so keeping it lazy buys nothing; a `u64` is O(1) to re-read. *(Alt: also
  expose the raw span for the rare consumer that wants the exact byte encoding — add on demand, not by
  default.)*
- **`I32`/`I64`** carry `C::Value` (the chunk view / span) so the raw LE bytes are available O(1) and
  `.as_u32()/.as_f32()/.as_u64()/.as_f64()` decode on demand (a fixed field almost always sits in one
  leaf → `as_slice()` is `Some`; the cross-leaf case reads ≤8 bytes via `chunks()`).
- **`Len`** carries `C::Value` — the payload bytes. This is the bulk content (strings, bytes, nested
  messages, packed arrays), the analog of etude-json's `String`/`Number` spans and where
  copy-avoidance pays. The consumer decides how to interpret it (§3.3).

### 3.2 The iterator

```rust
impl<'a, C: Capture> Iterator for Tokenizer<'a, C> {
    type Item = Result<Field<'a, C>, Error>;   // FusedIterator; None at clean end-of-input
}
```

Mirrors etude-json's `Tokenizer: Iterator<Item = Result<Token, Error>> + FusedIterator`
(`etude-json/src/lib.rs:568,:607`). `Tokenizer::new(input: &'a ByteVec)` (default `Chunks` policy);
`Tokenizer::<Spans>::spans(input)` for the opt-down policy.

### 3.3 Nested messages / packed fields — the schema-agnostic boundary

At the wire level a `Len` payload is opaque: string vs bytes vs embedded message vs packed repeated are
indistinguishable without a schema. So the tokenizer emits the raw `Len(value)` and the consumer
recurses if its schema says "message." Recursion is **cheap** because the value carries the position:

```rust
impl<'a, C: Capture> Tokenizer<'a, C> {
    /// Tokenize a LEN payload as a nested message — O(1) to spawn (no root descent;
    /// resumes from the value's captured cursor position / span).
    pub fn nested(value: &Value<'a, C>) -> Self;
}
```

Packed repeated fields (a `Len` payload that is a tight run of varints or fixed values) are likewise
consumer-driven: hand the payload bytes to a small `packed_varints(value)` / `packed_fixed` iterator.
These stay schema-agnostic helpers — the tokenizer never guesses.

## 4. Streaming across chunk boundaries (reuse the shared cursor)

The tokenizer runs on the **same** chunk-cursor `etude-json` proved and PR #101 extracts to
`etude-span` — `Cursor { chunks, chunk, pos, base }` with `peek/bump/skip_in_chunk/chunk_tail/
offset/refill` (`etude-json/src/lib.rs:244`). NO per-byte `byte_at` (which would be O(n log n),
cache-hostile — the exact regression etude-json fixed in its PR #77). Concretely:

- **Varint / tag:** read bytes via `peek`/`bump`; the LEB128 loop crosses a leaf boundary transparently
  because `bump` calls `refill` at the leaf end. Cap at 10 bytes → `Overlong` error past that.
- **`Len` payload:** record the start `offset()` (span start) + decode the length, then advance `len`
  bytes via `skip_in_chunk` (bulk within a leaf) + `refill` at boundaries — the same bulk skip
  etude-json used to make string scanning fast. Capture the value view: if the payload fits the current
  leaf (`chunk_tail().len() >= len`), emit `Bytes::Chunk{ slice: &chunk_tail()[..len], span }` (O(1));
  else emit `Bytes::Split(..)` capturing the first leaf tail + a resumable `Chunks` clone.
- **Fixed `I32`/`I64`:** read 4/8 bytes; same single-leaf-fast / cross-leaf-split capture.

Non-destructive: like etude-json, tokenize by walking `chunks()` directly (or over an O(1)-shared
`ByteVec::reader()` clone, `lib.rs:2399`) so the input rope is untouched and absolute offsets are kept
without descents. We do NOT read through `etude_buffer::reader::Buffer::read_chunk` — it *consumes* the
source (`etude-buffer/src/reader.rs:63`), wrong for a re-readable token stream.

**Truncation / malformed.** `etude-buffer`/`etude-bytevec` have no `NeedMore`/incomplete notion (a short
read just yields an empty chunk; `etude-buffer/src/error.rs:10`). The tokenizer introduces its own
byte-offset error, mirroring etude-json's `Error{kind, offset}` (`etude-json/src/lib.rs:202`):

```rust
pub enum ErrorKind {
    Truncated,          // tag / varint / fixed field / LEN payload runs past end-of-input
    OverlongVarint,     // varint > 10 bytes, or 10th byte sets bits above 63
    InvalidWireType,    // low 3 bits == 6 or 7 (only 0..=5 are defined)
    LengthOverflow,     // LEN length doesn't fit usize / overflows the remaining input
    // GroupMismatch is a CONSUMER concern (schema-agnostic tokenizer emits SGroup/EGroup faithfully)
}
```

## 5. Increments (top-to-bottom, the way a `vertical` lands them)

Each is one coherent, independently-green, direct-to-main PR in the etude repo (mirrors etude-json's
per-slice cadence). Since the operator is away, **decision-laden PRs open for GitHub review; mechanical
slices `--admin`-merge** (etude away-mode rule, etude-json log).

- **Slice 0 — shared value-ref in `etude-span` (co-owned with etude-json).** Add `Bytes<'a>`,
  `Split<'a>`, and the `Capture`/`Spans`/`Chunks` ZST policy to `etude-span` (on top of PR #101's
  `Span`+`Cursor` extraction). This RESOLVES etude-json's PR-#101 open question; coordinate so
  etude-json adopts the same type (its `String`/`Number` spans become `Bytes`). **Decision-laden →
  open PR for review.** Blocks the O(1) path of both tokenizers, so it lands first (or concurrently
  with etude-span's own landing).
- **Slice 1 — the wire tokenizer skeleton.** `crates/etude-protobuf`: varint/tag decode, the four
  value cases (`Varint`/`I64`/`I32`/`Len`), forward `Iterator`, byte-offset errors. Default `Chunks`
  policy but a span-only fallback compiles. `no_std`+alloc floor; builds `wasm32-unknown-unknown`
  (default + no-default-features) + `wasm32-wasip1` (etude-json's floor). Differential oracle (§6) +
  0-alloc assertion. **Mechanical foundation → `--admin` once green.**
- **Slice 2 — the chunk-view value access (the operator's O(1) ask).** Wire `Value` to `C::Value`;
  `Bytes::as_slice/chunks/span`; `as_u32/u64/f32/f64` for fixed fields. Bench proves O(1) re-access vs
  a span-only baseline that re-descends (§7).
- **Slice 3 — nested-message recursion + groups.** `Tokenizer::nested`, `packed_varints`/`packed_fixed`
  helpers, `GroupStart`/`GroupEnd` structural tokens. Still schema-agnostic.
- **Slice 4 — benchmark scoreboard vs a reference decoder.** `benches/tokenize.rs` + `BENCHMARKS.md`,
  TIME + ALLOCATIONS first-class (jemalloc-counted, etude-bytevec `benches/compare.rs` template). Then
  measure→optimize→re-measure the gaps.

## 6. Differential oracle (the correctness spine)

**DECIDED default reference:** **`prost`'s low-level `prost::encoding` module** — it is schema-agnostic
(`decode_key`, `decode_varint`, wire-type readers), so it can validate a *wire* tokenizer without a
`.proto` schema. *(Alt: the `protobuf` crate's `CodedInputStream`; keep as a second oracle if useful.)*

The bolero harness mirrors etude-json/etude-bytevec (`etude-bytevec/src/tests.rs` is the canonical
pattern: `#[derive(TypeGenerator)]` op/doc model + a flat-oracle model + every chunk layout):

- **Model → wire → tokens.** Generate a model message = a `Vec<Record>` where each `Record` is a random
  `(field_number, WireValue)` with `WireValue ∈ {Varint(u64), Fixed32([u8;4]), Fixed64([u8;8]),
  Len(Vec<u8>), GroupStart, GroupEnd}`. Encode it to bytes (hand-rolled or via `prost::encoding`), build
  a `ByteVec` from those bytes under **several chunk layouts (1/3/16/1024-byte leaves)** — the
  boundary-straddling test etude-json relies on — and assert `etude-protobuf` produces exactly the
  implied `Field` stream (numbers, wire types, decoded varints, payload bytes).
- **Arbitrary bytes → sound rejection + never panic.** Feed random `Vec<u8>`: if the tokenizer rejects
  as malformed, `prost::encoding` must also fail to decode it as a well-formed record stream (the sound
  direction), and the tokenizer must **never panic** on any input under any chunk layout.
- **Chunk-view invariant.** For every emitted value, `Bytes::span()` bytes (read via `ByteVec::slice`)
  MUST equal the bytes read via `Bytes::chunks()` — the O(1) path agrees with the portable path. Assert
  under every chunk layout (this is the etude-json "compare span bytes directly" discipline).

## 7. Benchmark scoreboard (the "then beat it" gate)

`benches/tokenize.rs` head-to-head vs the reference decoder (criterion, jemalloc alloc counting;
`etude-bytevec/benches/compare.rs` + etude-json `benches/tokenize.rs` templates). Axes: **allocations
first-class** (target: **0** — tokens are chunk-views/spans, like etude-json) AND time. Shapes:

- many small scalar fields (varint-heavy) · big `Len` bytes payload (the "big_string" analog, watch the
  bulk-skip path) · deeply-nested messages (recursion) · packed-repeated arrays · and the two access
  patterns etude-json settled on: **extract-a-few-fields-from-a-big-message** (where chunk-view O(1)
  access and skip-don't-decode should shine) vs **decode-everything**.
- Record ratios in `BENCHMARKS.md`; the O(1)-vs-O(log n) win from §2 should show as a flat read cost as
  message size grows, vs a span-only baseline whose per-token read cost climbs with `log n`.

## 8. Open decisions — all with a chosen default (for async GitHub review)

1. **Scope: wire-only, schema-agnostic** [default, §1] vs schema-driven decode. → wire-only; schema
   layer is a separate future crate.
2. **Token value representation: chunk-view `Bytes<'a>` embedding the span, default `Chunks` capture
   policy, `Spans` ZST opt-down** [default, §2] — the operator's ask. Alt (a) always-`Bytes`-no-policy,
   alt (c) span-only + `resolve_at` (subsumed as the `Spans` policy). **This is the operator-facing
   fork; flagged for review.**
3. **Varint: eager-decode to `u64`** [default, §3.1] vs lazy raw span. → eager (must decode to advance).
4. **Groups (wire 3/4): emit `GroupStart`/`GroupEnd` structural tokens** [default, §1] vs reject.
   → emit (faithful wire reader).
5. **Oracle reference: `prost::encoding` low-level** [default, §6] vs `protobuf` `CodedInputStream`.
6. **Fixed fields: carry raw bytes + decode-on-demand helpers** [default] — no eager LE decode.

## 9. Seams & file anchors (all in the etude repo unless noted)

- **New crate:** `crates/etude-protobuf/{src/lib.rs, src/tests.rs, benches/tokenize.rs, BENCHMARKS.md}`.
- **Shared primitive (co-owned with etude-json):** `crates/etude-span` — `Span` (PR #101), `Cursor`
  (PR #101), and the NEW `Bytes<'a>`/`Split<'a>`/`Capture`/`Spans`/`Chunks` (§2, Slice 0).
- **Cursor prior art to extract/reuse:** `Cursor{chunks,chunk,pos,base}` + `peek/bump/skip_in_chunk/
  chunk_tail/offset/refill` — `.claude/worktrees/*/crates/etude-json/src/lib.rs:244`; `chunk_tail`
  borrows the input `'a` (`:280`) — the zero-cost chunk-capture seam.
- **Byte-rope:** `etude_bytevec::ByteVec::{chunks (lib.rs:487), slice (:789), reader (:2399), byte_at
  (:430, O(log₃₂) — avoid per-byte)}`; `Chunks<'a>: ExactSizeIterator<Item=&Bytes>` (:1316).
- **Buffer trait (NOT used for reading — it consumes):** `etude_buffer::reader::Buffer::read_chunk`
  (`reader.rs:63`), `Chunk<'a>` leaf enum (`reader/chunk.rs:14`). Cited to explain why we walk
  `chunks()` directly instead.
- **Test/bench templates:** `etude-bytevec/src/tests.rs` (bolero + flat oracle + tiers),
  `etude-bytevec/benches/compare.rs` (criterion head-to-head + jemalloc), etude-json's
  `src/tests.rs`/`benches/tokenize.rs` (chunk-layout differential + alloc table).
- **This design doc** lives in cadenza `implementation/design/DESIGN-protobuf-tokenizer.md` (the fleet's
  design home); the code lives in the etude repo.

## 10. Coordination

- **etude-json** owns landing `etude-span`. This design's Slice 0 (`Bytes`/policy) is the ADDITION that
  resolves etude-json's PR-#101 open question; both tokenizers adopt one shared value-ref. A `note` to
  etude-json accompanies this doc with the agreed `Bytes<'a>` + `Capture` API so it lands the shared
  type once.
- **etude-byterope-compat / fixer-byterope** own `etude-bytevec`; this design only *consumes* its public
  `chunks()`/`slice()`/`reader()` — no changes requested. (If the split-value `Chunks`-clone bookmark
  wants a cheaper clone than the current walk-stack, that is a possible future `note` to them, not a
  blocker — the O(depth) clone is already fine.)
- **The `vertical` owner** builds top-to-bottom in the etude repo, comms home a cadenza worktree, exactly
  like etude-json (`fleet/loops/etude-json.md` is the charter template).
- **Operator (async):** the §8.2 token-representation fork is the one decision surfaced for GitHub
  review; everything else has a chosen default noted with its alternative.
