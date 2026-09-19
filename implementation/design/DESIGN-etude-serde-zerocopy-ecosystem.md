# Design — etude: the zero-copy serde ecosystem (span · chunk-streaming reader · StrRope · rope-native Visitor)

**Author:** design-etude-serde-zerocopy (design partner). **Audience:** the etude cohort building the
copy-avoiding decode stack — `etude-byterope-compat` (Rope<K> repr), `etude-str-migration` (StrRope str
API), `etude-json` (first decoder), `etude-decimal`/`etude-bigint`/`etude-rational` (value types), and any
future decoder. **Repo:** the crates described here live in the `etude` repo (`etude/crates/etude-*`); this
doc is the shared ecosystem spec the builders converge on.

**Status:** DESIGN — synthesized from the operator's design inputs (span-in-its-own-crate; UTF-8 strrope
with seamless byte↔str interop; chunk-streaming reader; decoder-owns-parsing / value-crate-ingests-
validated-components) and the real build constraints reported by `etude-json` (slice 1 tokenizer) and the
`etude-str-migration`/`etude-byterope-compat` StrRope cohort. **The operator is away**; every forking
decision below is resolved with a chosen default (marked **DECISION**) aligned to the operator's demonstrated
priorities — aggressive zero-allocation, reuse of the shared rope/chunk cursor, tokens carrying spans (not
O(log n) indexing), minimal dependencies, fewer/cleaner APIs, correctness first. Genuinely open forks are
collected in §8 for async GitHub review; none of them block a builder from starting increment 1.

---

## 0. The one idea, and why stock serde does not fit

The ecosystem decodes structured formats (JSON first) out of a **byte rope** — `etude-bytevec::ByteVec`, a
deque of refcounted `bytes::Bytes` chunks whose `slice(range)` is an O(1) structural share (a refcount bump,
no copy). The whole point is **copy-avoidance**: a decoded "borrowed" value should be a cheap shared handle
into the source rope, materialized to an owned buffer only when the format forces it (an escaped string, a
decoded number).

Stock `serde` cannot express this. Its `Deserializer<'de>` borrows contiguous `&'de str` / `&'de [u8]`
slices of the input, threaded through every Visitor by a `'de` lifetime. That model breaks on a rope for two
independent reasons:

1. **A token is not contiguous.** A JSON string/number can straddle a rope-leaf (chunk) boundary
   (`etude-json` constraint 1), so its bytes are *not* a single `&[u8]` into one chunk. There is no `&'de
   str` to borrow.
2. **The rope already gives us zero-copy for free — via structural sharing, not lifetimes.** `ByteVec::slice`
   returns an *owned-but-shared* sub-rope in O(1). So the "borrow" we want is a refcounted rope slice, not a
   lifetime-bound reference.

**THE CENTRAL DECISION (DECISION-0): drop the `'de` lifetime. The zero-copy substrate is structural sharing,
not borrowing.** A "borrowed" value is an O(1) rope slice (an owned `StrRope`/`ByteVec`/`Span` that shares
the source's `Bytes` chunks), not a `&'de` reference. This removes the `'de` lifetime parameter from the
entire Deserializer/Visitor surface — *cleaner* than serde, and it is the only model that works over a
non-contiguous rope. Everything else in this design follows from DECISION-0.

---

## 1. The crate stack (bottom → top) and the boundaries

```
  etude-bytevec  ── ByteVec (byte rope) + Chunks/Reader cursor + Rope<K> repr        [foundation, exists]
       │  owns: chunk-streaming scan cursor (§3), Rope<K> ZST-marker repr (§4)
       ├── etude-span ─ Span = source-relative half-open byte range {start,end}      [NEW, tiny, no deps]
       │        resolve(&ByteVec) -> ByteVec | resolve_str(&StrRope) -> StrRope
       ├── etude-str (StrRope = Rope<Utf8>) ─ validated-UTF8 view, free ↔ ByteVec    [StrRope migration]
       │
  etude-serde  ── rope-native Value / Visitor / Deserializer traits (NO 'de)         [NEW, my namesake]
       │  the copy-avoidance value model: RopeStr (borrowed-slice | owned), Span-carrying number tokens,
       │  the Visitor contract, the decoder↔value-type construction boundary (§5,§6)
       │
  decoders: etude-json (first), <future formats>  ── own grammar scan + validation   [consumers]
  value types: etude-decimal, etude-bigint, etude-rational ── construct-from-validated-components (§6)
```

**Dependency directions (no cycles):**
- `etude-span` depends on **nothing** (it is two `usize`s + helpers). `resolve`/`resolve_str` take the rope
  as an argument, so span does not depend on `etude-bytevec`; a thin `SpanExt` extension trait in
  `etude-bytevec` (or a `bytevec`-feature on `etude-span`) provides the ergonomic `rope.slice_span(span)`.
  **DECISION-1a: `etude-span` is dependency-free; the ergonomic resolver lives as an extension trait in
  `etude-bytevec` behind a default feature.** This keeps span reusable by non-rope callers and avoids a
  bytevec→span→bytevec cycle.
- The **chunk-streaming scan cursor lives IN `etude-bytevec`** (it already owns `Reader<'a>` and
  `Chunks<'a>`), extended with absolute-offset tracking + span emission (§3). **DECISION-1b: do NOT create a
  separate `etude-scan` crate** — fewer crates, and the cursor is intrinsic to the rope it walks.
- `etude-serde` depends on `etude-bytevec` + `etude-span` + `etude-str`. Decoders depend on `etude-serde` +
  the value types. Value types (`etude-decimal` etc.) depend on **`etude-span`/`etude-bytevec` only for the
  validated-component constructor inputs** — never on a decoder (§6).

---

## 2. `etude-span` — the foundational span primitive (operator-commissioned)

The operator: *"Can we put the span struct in a separate crate? We are going to have a lot of decoders."* The
span is the primitive every decoder records while scanning and every value layer resolves on demand.

**DECISION-2 (shape): a source-relative, half-open byte range — `Span { start: usize, end: usize }` — NOT a
borrowed `&[u8]`, NOT an eager sub-rope handle.** This is exactly what `etude-json` slice 1 validated in
practice (constraint 2): a `{offset,end}` range resolved every token cleanly and *defers* the actual sub-rope
split to the consumer, which is cheaper than pre-splitting a sub-rope per token.

Why offset+len beats the alternatives:
- **vs `&[u8]`:** a `&[u8]` cannot represent a cross-chunk region (constraint 1). A source-relative range
  represents a cross-chunk region *trivially* — `ByteVec::slice(start..end)` already stitches chunks. No
  special "cross-chunk span" representation is needed; that complexity simply does not exist in this model.
- **vs an eager sub-rope handle (`ByteVec`):** recording a range is two `usize` writes; materializing a
  sub-rope is an O(1)-but-nonzero refcount/structure op. Most tokens are skipped or decoded-in-place; only
  the ones a consumer *keeps* should pay the slice. So **materialize on demand.**

```rust
// etude-span (no dependencies)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Span { pub start: usize, pub end: usize }   // half-open, source-relative
impl Span {
    pub fn len(&self) -> usize { self.end - self.start }
    pub fn is_empty(&self) -> bool { self.start == self.end }
    pub fn shrink(&self, front: usize, back: usize) -> Span { .. }   // e.g. strip the quotes of a string
    pub fn range(&self) -> core::ops::Range<usize> { self.start..self.end }
}
// etude-bytevec, extension trait (DECISION-1a):
pub trait SliceSpan { fn slice_span(&self, s: Span) -> Self; }
impl SliceSpan for ByteVec { fn slice_span(&self, s: Span) -> ByteVec { self.slice(s.range()) } }
// (and the same for StrRope, char-boundary-checked — see §4/§5)
```

`Copy` + `no_std`-friendly (two `usize`s). **DECISION-2a: `Span` is `Copy` and carries no rope reference** —
it is an index-pair the caller resolves against the *known* source rope. Carrying a rope handle in every span
would defeat the "cheap to record" goal and re-introduce the lifetime problem DECISION-0 removed.

---

## 3. The chunk-streaming scan cursor (foundational, in `etude-bytevec`)

The operator flagged (and `etude-json` constraint 5 confirms): an absolute-offset cursor calling
`ByteVec::byte_at` is **O(log n) per byte** over the RRB rope. The scanning fast path must be a **cursor over
contiguous chunks with a local pointer** — O(1) per byte — that carries across chunk boundaries and *records
absolute offsets* so tokens come out as `Span`s.

**DECISION-3: extend the existing `etude-bytevec::Reader`/`Chunks` into a `ScanCursor` that (a) yields the
current contiguous chunk as `&[u8]` + its base absolute offset, (b) advances a local pointer within the chunk
(O(1)/byte), (c) carries across chunk boundaries transparently, and (d) exposes `pos() -> usize` (absolute)
so a decoder brackets a token as `Span { start: pos_before, end: pos_after }`.** It reuses `chunks()`,
`starts_with`/`ends_with`, and adds a from-offset compare + find-byte for literal/delimiter matching without
linearizing the rope.

Contract sketch (the decoder-facing scanning surface):
```rust
pub struct ScanCursor<'a> { /* wraps Chunks<'a> + intra-chunk index + absolute base */ }
impl<'a> ScanCursor<'a> {
    pub fn pos(&self) -> usize;                       // absolute offset, for Span bracketing
    pub fn peek(&self) -> Option<u8>;                 // current byte, O(1)
    pub fn bump(&mut self) -> Option<u8>;             // advance one byte, O(1), cross-chunk carry
    pub fn current_chunk(&self) -> &'a [u8];          // contiguous run from pos to chunk end (bulk scan)
    pub fn advance(&mut self, n: usize);              // skip n bytes (skippable numbers/whitespace)
    pub fn find(&self, byte: u8) -> Option<usize>;    // absolute offset of next `byte`, chunk-aware
    pub fn matches_at(&self, prefix: &[u8]) -> bool;  // literal match across chunks (true/false/null)
}
```
Bulk content is scanned via `current_chunk()` (contiguous `&[u8]` — SIMD/`memchr`-friendly) with cross-chunk
carry, never via per-byte random access. The `'a` here is a borrow of the *source rope during the scan* — it
is a scan-local lifetime, entirely separate from DECISION-0 (the produced *values* carry no `'de`).

---

## 4. `StrRope` — UTF-8 rope with free byte↔str interop (cohort-owned; my constraints)

`StrRope` is being built by `etude-str-migration` (str API + invariant) and `etude-byterope-compat` (the
`Rope<K>` repr + ZST `Utf8` validator). The agreed shape (from their notes): `StrRope = Rope<Utf8>`, a newtype
over the *same* generic `Rope<K>` as `ByteVec = Rope<Bytes>`, where `K` is carried purely as a zero-size
`PhantomData<K>` marker — so `Rope<Utf8>` and `Rope<Bytes>` are byte-identical in layout.

- **Invariant (operator ruling, relayed by etude-str-migration):** *concatenated* content is valid UTF-8;
  codepoints MAY span chunk boundaries (chunk boundaries are arbitrary — they arrive on syscall/network
  frames). So UTF-8 validity is a **whole-value** invariant, NOT per-chunk. A seam-stitching char iterator
  serves `&str`/chars across chunk boundaries; the data model treats a StrRope's bytes as an ordinary byte
  rope for framing/zero-copy.
- **Conversions:** `StrRope::into_bytes(self) -> ByteVec` is a **free field-move** (drop the ZST marker; valid
  UTF-8 is valid bytes). `ByteVec::try_into_str(self) -> Result<StrRope, _>` is O(n) validated.

**What `etude-serde` needs from this layout — the answer to `etude-byterope-compat`'s ask (also §7):**
1. **Free `StrRope → ByteVec`:** YES, required. The framing/serialize path treats a `StrRope` value exactly
   as its `ByteVec` bytes. Under `PhantomData<K>` this is inherently free — a field-move `into_bytes`, no
   copy. **I do NOT need a `transmute`; the safe field-move is sufficient and preferred.**
2. **No `repr(transparent)`/`repr(C)` required.** My zero-copy path never transmutes between `Rope<Utf8>` and
   `Rope<Bytes>` — it uses the safe `into_bytes()` field-move and the shared `slice`/`chunks`/`ScanCursor`
   API. So `repr(Rust)` default is fine; requiring `repr(transparent)` would over-constrain the repr for no
   benefit to me. (If the cohort later wants a `&StrRope → &ByteVec` *reference* transmute, that WOULD need
   `repr(transparent)` — but see (4): I only need a safe borrow-projection, so this stays optional.)
3. **`StrRope::slice(range) -> StrRope` must be O(1) and structurally share the source's `Bytes` chunks**
   (refcount bump only), identical to `ByteVec::slice`, and **char-boundary-checked** (return `Result`, or a
   `slice_unchecked` for a decoder that already knows the bytes are a whole no-escape token). This O(1)
   char-safe slice IS the zero-copy substrate for a "borrowed" string value (§5).
4. **A borrow-projection `StrRope::as_bytes(&self) -> &ByteVec`** (zero-cost, safe under `PhantomData`) so a
   decoder can run byte-level `ScanCursor` scanning over a `StrRope` input without consuming it.

**Constraint I'm asking the cohort to hold:** keep `PhantomData<K>` the **only** kind-carrying field (no
discriminant, no extra field) and expose `into_bytes`/`as_bytes`/char-safe `slice`. No repr attribute needed.

---

## 5. `etude-serde` — the rope-native value model and Visitor (my crate)

Per DECISION-0, no `'de`. The borrowed-vs-owned choice is a Cow over rope slices:

```rust
// etude-serde
pub enum RopeStr {
    Borrowed(StrRope),   // O(1) char-safe slice of the source rope — zero copy
    Owned(String),       // materialized: escaped string unescaped into an owned buffer
}
pub enum RopeBytes {
    Borrowed(ByteVec),   // O(1) sub-rope
    Owned(Vec<u8>),
}
```

- **Strings** (the copy-avoidance payoff — JSON's only bulk content): a no-escape string is
  `RopeStr::Borrowed(strrope.slice_span(inner_span))` — zero copy. An **escaped** string MUST materialize
  (`RopeStr::Owned`) — unescape can never be a pure borrow (constraint 3). The tokenizer records a cheap
  `has_escapes` flag during its mandatory scan so the value layer picks the path with **no re-scan**.
- **Numbers** are **always a decode, never a borrow, but skippable** (constraint 4). A number token is a raw
  `Span` + cheap `int/frac/exp` flags; the value is produced on demand by handing the span's digit/sign/exp
  components to a value-type constructor (§6). Lazy decode wins when a consumer skips fields.

**The Visitor contract (no `'de`, rope-shaped):**
```rust
pub trait Visitor {
    type Value;
    fn visit_str(self, s: RopeStr) -> Result<Self::Value, Error>;      // O(1)-shared or owned
    fn visit_bytes(self, b: RopeBytes) -> Result<Self::Value, Error>;
    fn visit_number(self, tok: NumberToken) -> Result<Self::Value, Error>;   // NumberToken = span + flags
    fn visit_bool(self, b: bool) -> Result<Self::Value, Error>;
    fn visit_null(self) -> Result<Self::Value, Error>;
    fn visit_seq<A: SeqAccess>(self, seq: A) -> Result<Self::Value, Error>;
    fn visit_map<A: MapAccess>(self, map: A) -> Result<Self::Value, Error>;
}
pub trait Deserializer {   // NO <'de> parameter
    fn deserialize_any<V: Visitor>(self, v: V) -> Result<V::Value, Error>;
    // ... typed hints (deserialize_str/map/seq/...) as serde has, minus the lifetime
}
```
`SeqAccess`/`MapAccess` yield sub-`Deserializer`s positioned by the `ScanCursor`; a map key is a `RopeStr`
(usually `Borrowed`). Because a `Borrowed` value is an owned-shared handle, it can outlive the scan with no
lifetime threading — the source chunks stay alive via refcount as long as any slice holds them (the same
"holds the input alive" tradeoff cadenza-ast's zero-copy decode already accepts; documented, not hidden).

---

## 6. The decoder ↔ value-type boundary (operator principle)

Operator (from the `etude-decimal #64` concern — it had embedded the JSON-number grammar in the decimal
type): *"I do not love doing parsing in the data structure — it should be done by the parser."*

**DECISION-6: decoders/parsers own grammar scanning + validation; value crates expose CONSTRUCT-FROM-
VALIDATED-COMPONENTS constructors, NOT format-grammar parsers.** The decoder scans a number token into its
components (sign, integer-digits span, fraction-digits span, exponent) and hands those *validated* components
straight to the value constructor — **no intermediate `String` allocation** (reinforcing copy-avoidance).

```rust
// etude-decimal exposes (NO json-number grammar inside):
impl Decimal {
    pub fn from_components(sign: Sign, int_digits: &[u8], frac_digits: &[u8], exp: i64)
        -> Result<Decimal, DecimalError>;    // digits are validated ASCII 0-9, guaranteed by the decoder
}
// the JSON decoder (etude-json) owns the grammar and calls the above; it may read digits chunk-aware
// from the ScanCursor, so even a cross-chunk number needs no linearized String.
```
Same shape for `etude-bigint` (`from_ascii_digits`) and `etude-rational`. A value type NEVER re-parses format
grammar; the `NumberToken`'s flags/spans are the contract between §5's Visitor and these constructors.

---

## 7. Increments (top-to-bottom, the way a vertical lands them)

Each increment is independently landable and gated; a later one depends only on earlier ones.

- **Increment 1 — `etude-span` crate.** The `Span` type (§2) + tests (record/shrink/range; cross-chunk
  resolve via `ByteVec::slice`). Gate: `cargo test -p etude-span`. Dependency-free; unblocks everyone.
- **Increment 2 — chunk-streaming `ScanCursor` in `etude-bytevec` (§3).** Extend `Reader`/`Chunks` with
  absolute-offset tracking, `current_chunk`, `find`, `matches_at`. Gate: cursor unit tests incl. a token
  bracketed across a forced chunk boundary; the `SliceSpan` extension trait (DECISION-1a). Benchmark: O(1)/
  byte scan vs the O(log n) `byte_at` baseline (validates the perf claim — principle 3, a measurable goal).
- **Increment 3 — `StrRope` free interop surface (cohort, my constraints §4).** `into_bytes` (free),
  `as_bytes` borrow-projection, char-safe O(1) `slice`/`slice_span`. Gate: round-trip `ByteVec ↔ StrRope`
  free/validated; slice shares chunks (assert refcount, not copy); char-boundary rejection. Reviewed by
  etude-str-migration (invariant) + etude-byterope-compat (repr).
- **Increment 4 — `etude-serde` value model + Visitor traits (§5).** `RopeStr`/`RopeBytes`/`NumberToken`, the
  `Visitor`/`Deserializer`/`SeqAccess`/`MapAccess` traits (no `'de`). Gate: a trivial in-memory decoder
  exercising borrowed vs owned string, skipped number, cross-chunk map key.
- **Increment 5 — value-type validated-component constructors (§6).** `Decimal::from_components`,
  `BigInt::from_ascii_digits`, etc.; migrate any grammar currently in a value type out to its decoder (closes
  the `etude-decimal #64` concern). Gate: constructor tests from raw component slices; assert no `String`
  alloc on the number path.
- **Increment 6 — `etude-json` adopts the model.** Its slice-1 tokenizer already emits `{offset,end}` +
  `has_escapes`; retarget it onto `etude-serde`'s Visitor and the `ScanCursor`, and route numbers through
  Increment 5's constructors. Gate: JSON conformance corpus; a benchmark showing zero allocation on a
  no-escape-string / skipped-number workload.

---

## 8. Open questions for async GitHub review (operator is away — none block increment 1)

Defaults are chosen (above); these are the forks worth an operator eyeball on the PR:

1. **DECISION-0 (drop `'de`, structural-sharing borrow).** Confirm we are content abandoning serde
   source-compat in favor of a rope-native, lifetime-free Deserializer. (Default: yes — it is the only model
   that works over a non-contiguous rope, and it is cleaner.)
2. **Crate for the value model.** Named `etude-serde` here. Alternatives: `etude-decode`, or fold the traits
   into `etude-bytevec`. (Default: a separate `etude-serde` — decoders/value-types depend on the trait crate
   without pulling each other.)
3. **`Span` as bare `{usize,usize}` vs a rope-tagged span.** Default bare + `Copy` (DECISION-2a). Revisit only
   if a decoder is found that resolves spans against *multiple* source ropes simultaneously (none today).
4. **`repr(transparent)` on `Rope<K>`.** I do NOT require it (§4.2); flagged so the cohort does not add it
   speculatively. Confirm the safe `into_bytes` field-move + `as_bytes` projection are acceptable in place of
   a reference transmute.
5. **"Holds the input alive" tradeoff.** A `Borrowed` value keeps the source rope's chunks alive via refcount
   (same as cadenza-ast zero-copy decode). Acceptable for streaming decode; a `.into_owned()` escape hatch on
   `RopeStr`/`RopeBytes` lets a long-lived consumer detach. (Default: provide `into_owned`, document the
   tradeoff.)

---

## 9. File/seam anchors

- `etude/crates/etude-span/` — NEW crate (increment 1).
- `etude/crates/etude-bytevec/src/lib.rs` — `ByteVec::slice` (L789), `chunks()` (L487), `Reader` (L2411),
  `starts_with`/`ends_with` (L576/L611), `byte_at` (L430, the O(log n) path to avoid in bulk scan); ADD
  `ScanCursor` + `SliceSpan` (increments 2/1a); `Rope<K>` repr + `into_bytes`/`as_bytes` (increment 3, cohort).
- `etude/crates/etude-str/` — StrRope str API + char-safe `slice` (increment 3, etude-str-migration).
- `etude/crates/etude-serde/` — NEW crate: value model + Visitor/Deserializer (increment 4).
- `etude/crates/etude-decimal/`, `etude-bigint/`, `etude-rational/` — `from_components`/`from_ascii_digits`
  validated-component constructors (increment 5).
- `etude/crates/etude-json/` — retarget the slice-1 tokenizer onto the model (increment 6).
