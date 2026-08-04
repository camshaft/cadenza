# Design — binary-AST dictionary (hashed dict-imports + node-by-index, hermetic transport codec)

**Author:** design agent (`design-ast-dictionary`). **Audience:** `v-syntax` (owns the wire half in
`cadenza-ast`), `v-metaprogramming` (owns the model/resolution side), `v-agent-harness` (the first
consumer — the AST-as-ABI invoke primitive), + future me.
**Status:** design DECIDED (option A + seq-125 refinement) and BUILD UNDERWAY — **I1 (the `cdzast\x00\x02`
transport plane + `decode_with_dicts`) LANDED** on trunk (v-syntax, PR #2086). This doc pins the shape,
the increments (top-to-bottom the way a vertical lands them), the seams/file anchors, the gate, and the
deferred extensions with a chosen default. This revision folds the operator's seq-125 clarification, the
Hash-crate-location fix, and the six PR-#2082 design-clarity review points.

The operator floated this across three Slack messages (seq 119–121, verbatim in §1). It is a coherent,
meaningful feature that COLLIDED with a spec-pinned invariant, so it got a design pass with the operator
before any frozen-wire code was cut; `v-syntax` held all dict wire code pending the ruling, then cut I1.

---

## 1. The feature (operator, seq 119–121, verbatim)

- **seq 119:** "One thing that would be really nice is to be able to have a dictionary of leaf values
  for the binary AST. So if we had a section of hashed dictionary imports and then you could refer to an
  indexed dictionary with the leaf index. That would make the actual ast encoding very compact while
  still allowing for evolution of the dictionary."
- **seq 120:** "And the compiler would take the dictionaries as an input artifact so it could properly
  resolve those without making any external calls or anything."
- **seq 121:** "And I think a dictionary could really just be another binary AST, actually! So it's not
  even strictly limited to leaf values - it could be any arbitrary AST node!"
- **seq 122 (the ruling):** "I won't be able to do a design session. But I think any ast that has a
  dictionary is non-canonical and that's fine." → **option A** (a dict-bearing AST is non-canonical;
  the one canonical encoding of any AST stays inline; content-addressing + `Event::hash` untouched).
- **seq 125 (identity refinement):** "To be clear on the dictionaries - their identities would be the
  hashes of the contents. So ASTs would be DAGs and hashing them would just be hashing the hashes of the
  dictionaries. You wouldn't need to inline or even deref any of them. We could always provide a way to
  fully deref and canonicalize as well."

**So the feature.** A binary-AST encoding carries a SECTION of hashed dictionary IMPORTS
(content-addressed). A node — **any arbitrary AST subtree, not just a leaf** (seq-121, since a
dictionary is itself just another binary AST) — can be encoded as an INDEX into an imported dictionary.
Dictionaries are supplied to the compiler/decoder **as input artifacts** (seq-120) and resolved
**hermetically** — NO external calls, ever. Goals: very compact AST encoding (repeated subtrees → one
index) and dictionary EVOLUTION (versioned by content hash).

---

## 2. The load-bearing fork, and the operator's ruling

`spec/contracts/ast-encoding.md` (FROZEN CONTRACT) pins a **bijection**:

> Each abstract syntax tree MUST have exactly one canonical binary encoding.
> Two abstract syntax trees that are equal MUST have identical binary encodings.
> Decoding a canonical binary encoding MUST yield the abstract syntax tree it was encoded from.

Content-addressing and the kernel durable log (`cdz-kernel/src/event_ast.rs` encodes every `Event`
through the ONE shared codec, header `cdzast\x00\x01`) depend on this bijection. The dictionary
introduces a SECOND way to encode the same tree (inline vs by-index), which tensions the pin.

`v-syntax` surfaced three options; the operator RULED:

- **(A) — CHOSEN (seq 122). A dict-bearing AST is NON-CANONICAL — "and that's fine."** The ONE
  canonical (inline `cdzast\x00\x01`) encoding of any AST is UNCHANGED; the frozen bijection holds over
  the inline plane exactly as today. A dict-bearing artifact is a legitimate, non-canonical FORM of the
  same tree that can always be fully DEREFERENCED + canonicalized back to that one inline encoding.
- (B) — rejected. Canonicality becomes dict-relative (the inline bijection itself is redefined). SPEC
  CHANGE to the frozen pins. Not needed — A gets the compaction without touching the inline contract.
- (C) — rejected. Defer entirely.

**The seq-125 refinement — the DAG form is DIRECTLY content-addressable (no deref needed).** The
operator sharpened A: dictionaries are content-addressed (identity = the content hash of the dict), and
**a dict-bearing AST is a DAG whose content hash is a HASH-OF-HASHES** — you hash the AST's own
structure together with the content-hashes of the dictionaries it references, **without inlining or even
dereferencing them**. So a dict-bearing artifact is NOT identity-less: it has a well-defined content
address computed cheaply over the DAG. Full-deref-to-inline + canonicalize is ALWAYS AVAILABLE as a
transform, but it is NOT on the hashing hot path. This is a strengthening: dictionaries buy both
compaction AND a cheap structural content-hash, and two equal DAGs (same structure + same referenced
dict-hashes) hash equal WITHOUT any deref.

**Consequence — TWO content-address bases over ONE tree, related by an available transform:**

| form | header | content hash | is it THE canonical bijection form? |
|---|---|---|---|
| **inline** | `cdzast\x00\x01` | hash of the canonical inline bytes (unchanged) | **YES** — the frozen bijection; corpus + `Event::hash` unchanged |
| **DAG / dict-bearing** | `cdzast\x00\x02` | **hash-of-hashes**: own structure + referenced dict content-hashes, NO deref | **NO** — a non-canonical form; deref+canonicalize → the inline form |

Both forms have a well-defined content address; they are the SAME tree in two forms, bridged by the
`deref + canonicalize` transform (DAG → inline). Which address a given SUBSYSTEM keys on is that
subsystem's choice (§2.1). Dict-free bytes stay `cdzast\x00\x01` **byte-identical to today** — the
entire existing corpus and every stored artifact are untouched, and `codec::encode`/`decode`/`canon` are
not modified.

### 2.1 Which hash does each subsystem key on?

- **The kernel durable log / `Event::hash` / the existing corpus keep the INLINE hash, unchanged.** The
  frozen bijection is guarded (§7.2). No existing identity moves. This is the hard invariant of A.
- **A dict-bearing artifact (the invoke wire, at-rest transfer) may be content-addressed by its DAG
  hash-of-hashes** — cheap, no deref, and stable given a fixed referenced dict-set. This is what makes
  dictionaries pay off on the hot path (you neither expand nor re-hash the shared subtrees).
- **The two are bridged, not merged:** `deref + canonicalize` maps a DAG to its inline form (and inline
  hash); there is no requirement that the DAG hash equals the inline hash (it does not — different
  bytes, different bases). A subsystem that needs the canonical identity of a dict-bearing artifact runs
  the transform; one that only needs a stable structural address uses the DAG hash directly.

**One point flagged to the operator via the concierge (not blocking):** whether the STORED/primary
identity of a dict-bearing program should be its DAG hash-of-hashes or its deref-canonical inline hash.
seq-125 leans DAG-hash-as-primary ("you wouldn't need to inline or even deref"); this doc adopts that as
the default (DAG hash is a first-class content address; deref-canonical is the available transform), and
the inline hash remains the FROZEN identity for everything that exists today. If the operator wants the
stored identity of NEW dict-bearing programs pinned to the deref-canonical hash instead, that is a
one-line change to §2.1 — it does not alter the wire or the increments.

---

## 3. Current ground truth (file/line anchors)

All in `implementation/seed/crates/cadenza-ast/src/` unless noted.

**The arena.** `ast.rs` — `Arenas { leaves: Vec<Leaf>, structure: Vec<Struct>, root: StructId }`.
`Struct` is `Atom(LeafId)` (a leaf) or `List(Vec<StructId>)` (an ordered child sequence). A NODE is a
`StructId` into `structure`; the tree is `structure[root]` walked recursively.

**The wire** (`codec.rs` module header, lines 1–79). Layout:
```text
[ header:8 = "cdzast\x00\x01" ]
[ leaf_count:var ] then each leaf: [ kind:1 ][ payload ]
[ struct_count:var ] then each entry: [ tag:1 ] Atom→[leaf_id:var] | List→[n:var][child_id:var]*
[ root:var ]                          a StructId
```
`TAG_ATOM=0`, `TAG_LIST=1` (`codec.rs:110–111`). `SCHEMA_HEADER = *b"cdzast\x00\x01"` (`codec.rs:159`).

**encode** (`codec.rs:179`) canonicalizes first (`canon::canonicalize`, `canon.rs:30`) then straight-
walks the two vectors — equal trees → identical bytes. **decode** (`codec.rs:308`) → `decode_detailed`
(`codec.rs:317`): verifies the header, referential integrity (ids in range, `codec.rs:341/373`), that
the reachable structure is a genuine TREE (no cycle / no shared subtree — a decode-bomb guard,
`codec.rs:391–405`), and no trailing bytes (`codec.rs:408`). Total: never panics, never returns a wrong
tree. `DecodeError` (`codec.rs:120`) classifies WHY (`Truncated` = torn write vs everything-else =
corruption).

**Content-addressing / durable log.** `cdz-kernel/src/event_ast.rs` maps each `Event` to `Arenas` and
encodes through THIS codec (header `cdzast\x00\x01`) — the durable log IS this canonical form. Nothing
in the dict feature may perturb the bytes this path produces.

**Why a dict-ref is naturally a new STRUCTURE ENTRY tag.** A node is a `StructId`; a dict-ref replaces
a subtree with "go fetch node `j` from imported dict `i`". So the clean seam is a THIRD `Struct`
variant on the transport plane only — `DictRef { dict: u32, node: u32 }` — carried by a new entry tag
`TAG_DICT_REF=2` in the `cdzast\x00\x02` structure section. It sits exactly where an `Atom`/`List`
would, so ANY subtree position (leaf or interior) can be a dict-ref — satisfying seq-121 (arbitrary
node, not just a leaf) for free. The existing `TAG_ATOM`/`TAG_LIST` bytes are unchanged.

---

## 4. The shape (decided)

### 4.1 A dictionary is a content-addressed inline-canonical AST

A dictionary IS just another binary AST (seq-121): a normal `cdzast\x00\x01` inline-canonical byte
string. Its content hash (a value-only `Hash([u8;32])` over the canonical bytes, defined in
`cadenza-ast` — see §9.1 for why it lives in the bottom crate, not `cdz-kernel`) is its identity. A
dict's importable NODES are the `StructId`s of its own
`structure` arena: dict-ref `{dict: i, node: j}` resolves to "the subtree rooted at `structure[j]` of
the dictionary whose hash is the `i`-th import".

**Decided: dictionaries are FLAT in v1.** A dictionary's bytes MUST be inline-canonical
(`cdzast\x00\x01`, dict-free). A dictionary does NOT itself carry dict-imports. Rationale: cycles are
IMPOSSIBLE by construction (dict bytes carry no imports → the resolver is a single flat expand pass with
no cycle-guard needed), and layering is a clean ADDITIVE v2 extension (see §8) if a real need appears.
This keeps v1's resolver a bounded, obviously-terminating graft.

### 4.2 The transport wire (`cdzast\x00\x02`)

```text
[ header:8 = "cdzast\x00\x02" ]
[ import_count:var ] then each import: [ hash:32 ]   # content hashes, sorted ASCENDING lexicographically by the 32 raw bytes
[ leaf_count:var ]  then each leaf (identical leaf encoding to v1)
[ struct_count:var ] then each entry: [ tag:1 ]
      TAG_ATOM(0)      → [ leaf_id:var ]                 # unchanged
      TAG_LIST(1)      → [ n:var ][ child_id:var ]*      # unchanged
      TAG_DICT_REF(2)  → [ dict_idx:var ][ node_id:var ] # NEW: node_id into import[dict_idx]'s arena
[ root:var ]
```
The import section is ORDERED canonically — imports sorted ascending by the 32 raw hash bytes,
lexicographically — so that a dict-bearing artifact ALSO has a deterministic byte form GIVEN a fixed
ref-set (useful for de-dup/caching of transport artifacts, though per A this is NOT a program-identity
claim). **`dict_idx` indexes THIS sorted import list** (the one and only import table — there is no
second, differently-ordered table for `dict_idx` to disagree with), so a `DictRef`'s `dict_idx` names
`import[dict_idx]`'s hash directly. `node_id` indexes the referenced dictionary's own `structure`
arena. Both `dict_idx` and `node_id` are bounds-checked on decode. Note the sort is a property the
ENCODER establishes; a decoder does NOT require the imports to be sorted to resolve refs (it just reads
`import[dict_idx]`), so an out-of-order import list is still decodable — sorting only guarantees the
deterministic-bytes property for a producer that wants it.

### 4.3 Hermetic resolution — `decode_with_dicts`

```rust
/// A resolved set of importable dictionaries, keyed by content hash. Supplied to the decoder as an
/// INPUT ARTIFACT (seq-120) — the decoder makes NO external calls; a hash not present is a hard error.
pub struct DictSet { /* hash -> decoded, inline-canonical Arenas (validated flat: dict-free) */ }

/// Decode a possibly-dict-bearing transport artifact against a supplied DictSet, EXPANDING every
/// dict-ref into the subtree it names, and returning a normal (dict-free) `Arenas`. A `\x00\x01` input
/// dispatches to `decode_detailed`; only `\x00\x02` engages the graft path. Total, like decode_detailed:
/// never panics, never returns a wrong tree. Does NOT canonicalize — feed the result to `encode` for that.
pub fn decode_with_dicts(bytes: &[u8], dicts: &DictSet) -> Result<Arenas, DecodeError>;
```
*(This section matches the LANDED `implementation/seed/crates/cadenza-ast/src/codec.rs` — I1, PR #2086/#2093.)*
- `cdzast\x00\x01` input → dispatches to `decode_detailed` (the `dicts` are unused); the two never
  disagree on a dict-free artifact. (It calls `decode_detailed`, not `decode`, so it likewise CLASSIFIES
  the error rather than dropping it.)
- `cdzast\x00\x02` input → resolve every import hash against `dicts` (missing → `MissingDict(Hash)`),
  bounds-check each `DictRef` (`dict_idx < import_count`, `node_id < that dict's struct_count`), apply the
  decode-bomb TREE GUARD to the transport structure BEFORE grafting (a cycle among the transport's own
  `List` ids would make the graft diverge — a `DictRef` is a leaf for this walk), then GRAFT: walk the
  transport structure post-order, COPY each `Atom`/`List`, and at each `DictRef` splice a FRESH COPY of
  the named dictionary's subtree, interning leaves by value into one deduped pool. The result is a normal
  dict-free `Arenas`, a genuine tree BY CONSTRUCTION (rebuilt fresh post-order), with a cheap defensive
  `verify_tree` re-check at the end.
- **There is NO post-graft `canon::canonicalize` inside `decode_with_dicts`** — the graft produces a
  well-formed dict-free arena, and canonicalization to the one normal form is imposed by `encode` /
  `canon::canonicalize`, NOT by the decoder (same as canonical `decode`, which also does not canonicalize
  on the way in). So the "full deref + canonicalize" transform the operator named (seq 125) is
  `decode_with_dicts` (deref) FOLLOWED BY `encode` (which canonicalizes) — not a single call.
- Getting the DEREF-CANONICAL identity is therefore: `encode(decode_with_dicts(bytes, dicts))` →
  canonical `cdzast\x00\x01` inline bytes. (Whether a subsystem keys on this or on the cheaper DAG hash
  of §4.5 is its choice — §2.1.)

**The canonical `decode` REFUSES `cdzast\x00\x02`.** Per A, dict-bearing bytes are non-canonical: the
identity-bearing `decode`/`decode_detailed` continue to accept ONLY `cdzast\x00\x01`. Precisely: today
`decode` returns `Option<Arenas>` (it DROPS the error reason) and `decode_detailed` returns
`Result<Arenas, DecodeError>` (it CLASSIFIES it) — this design does NOT change those signatures. So on a
`\x00\x02` header, `decode_detailed` returns `Err(DecodeError::BadHeader)` and `decode` returns `None`
(the `.ok()` of that `Err`) — the refuse-on-mismatch guarantee (`ast-encoding.md` §The Encoding Is
Versioned) holds through BOTH surfaces; "returns `BadHeader`" throughout this doc means the
`decode_detailed` classification, surfaced as `None` through `decode`. Only the explicitly-transport
`decode_with_dicts` (which returns `Result<Arenas, DecodeError>`) accepts `\x00\x02`. This is the
structural guarantee that a dict artifact can never be mistaken for an identity artifact.

### 4.4 The transport encoder — `encode_with_dicts` (honor-supplied-dict)

```rust
/// Encode `arenas` as a transport artifact that REFERENCES the supplied dictionaries: any subtree of
/// `arenas` that is structurally equal to an importable node of some dict in `dicts` MAY be emitted as
/// a DictRef instead of inline. v1 emits a ref for an EXACT subtree match against a caller-SUPPLIED
/// dict-set; it does NOT choose which subtrees to factor into a dictionary (that is dict CONSTRUCTION,
/// deferred — §8). Round-trips: decode_with_dicts(encode_with_dicts(a, d), d) == canonicalize(a).
pub fn encode_with_dicts(arenas: &Arenas, dicts: &DictSet) -> Vec<u8>;
```
(Naming: the pair is `decode_with_dicts` / `encode_with_dicts` — both PLURAL, since each takes a
`DictSet` of potentially many dictionaries. The symmetric names make the transport pair obvious next to
the canonical `decode`/`encode`.)
**Decided: v1 = decode/resolve + honor-supplied-dict.** v1 delivers the transport codec and an encoder
that emits refs against a dict-set the CALLER supplies. Automatic dictionary CONSTRUCTION (scanning a
corpus, choosing high-frequency/large repeated subtrees to factor into a dict, emitting `(dict, refs)`)
is a separate, later increment (§8). This keeps v1 small and PROVABLE — the round-trip and hermeticity
properties are the whole correctness story and are testable without a heuristic builder.

### 4.5 The DAG content-hash — `hash_dag` (hash-of-hashes, no deref)

Per seq-125, a dict-bearing artifact has a cheap content address computed WITHOUT dereferencing any
dictionary:

```rust
/// Content-hash a possibly-dict-bearing transport artifact as a DAG, WITHOUT resolving/dereferencing
/// its dictionaries: fold the artifact's own leaves + structure with the 32-byte content-hashes of the
/// dictionaries it imports (a hash-of-hashes). Requires only the import hashes ALREADY in the bytes — it
/// needs no DictSet and makes no external call. For a `cdzast\x00\x01` (dict-free) input this equals the
/// ordinary content hash of those bytes, so it agrees with the inline identity on dict-free artifacts.
pub fn hash_dag(bytes: &[u8]) -> Result<Hash, DecodeError>;
```
- Two DAGs with the same structure AND the same referenced dict-hashes hash equal — no deref.
- `hash_dag` of a dict-BEARING artifact does NOT equal the inline hash of its deref-canonical form (they
  are different byte bases). They are bridged by `decode_with_dicts` + `encode` (the deref-canonical
  transform), not by hash equality — §2.1.
- Because the import section is hash-sorted (§4.2) and leaves/structure are the canonical arena vectors,
  `hash_dag` is stable given a fixed ref-set. (v1 keeps this straightforward: it folds the bytes'
  own leaf/structure vectors + the sorted import hashes. A stronger "canonical-DAG" hash that is
  invariant under choice of which subtrees were factored is a v2 refinement — §8 — not needed for a
  stable address of a GIVEN encoding.)

### 4.6 The decode-error surface

Extend `DecodeError` (`codec.rs:120`) additively:
- `MissingDict(Hash)` — a `\x00\x02` artifact imports a hash NOT present in the supplied `DictSet`. This
  is the hermetic-resolution failure (seq-120: never fetch it — error out). Distinct from corruption.
- (Reserved for v2 layering, §8: `CyclicDict` — an import graph that is not a DAG.)

`DictRef` bounds violations (`dict_idx`/`node_id` out of range) reuse `IdOutOfRange`. A `\x00\x02`
whose grafted result is not a tree reuses `NotATree`. `Truncated`/`BadTag`/`MalformedVarint`/`BadText`/
`TrailingBytes` keep their meanings.

---

## 5. Increments (top-to-bottom, the way a vertical lands them)

Each increment is independently gate-green and a MEANINGFUL merge-request (a whole slice, not a drip).

- **I1 — transport container + decode/resolve (`v-syntax`, area=`cadenza-ast`).** Add the `\x00\x02`
  header constant, the `TAG_DICT_REF` structure tag, the `DictRef` transport variant (transport-plane
  only — NOT added to the identity `Struct` enum's canonical encoding), `DictSet`, `MissingDict(Hash)`,
  and `decode_with_dicts`. The canonical `encode`/`decode`/`canon` paths and `cdzast\x00\x01` bytes are
  UNTOUCHED. Gate: a dict-free `\x00\x02` decodes identically to `decode`; a `\x00\x02` with refs
  resolves + grafts + passes the tree guard; a missing hash → `MissingDict`; out-of-range ref →
  `IdOutOfRange`; canonical `decode_detailed` REFUSES `\x00\x02` with `Err(BadHeader)` (and `decode`
  with `None`) — signatures unchanged. This is the load-bearing slice.
- **I2 — transport encoder honoring a supplied dict (`v-syntax`).** Add `encode_with_dicts`: emit a
  `DictRef` for a subtree that EXACTLY matches an importable node of a supplied dict; else inline.
  Emit imports in canonical (hash-sorted) order. Gate: the ROUND-TRIP identity
  `decode_with_dicts(encode_with_dicts(a, d), d) == canonicalize(a)` for a matrix of trees × dict-sets
  (empty dict, matching dict, superset dict), AND `encode(decode_with_dicts(...)) == encode(a)`
  (transport is identity-preserving). A fuzz/property test over random arenas + random dicts.
- **I2b — the DAG content-hash `hash_dag` (`v-syntax`, folds into I2).** Add `hash_dag(bytes)` (§4.5):
  hash-of-hashes over the artifact's leaves/structure + its sorted import hashes, no deref. Gate: for a
  dict-free input `hash_dag(b) == content_hash(b)` (agrees with inline identity); two DAGs with the same
  structure + same import hashes hash equal; changing a referenced dict hash changes the DAG hash. This
  is the seq-125 cheap structural address; it is a small, coherent add on I2's encoder and can ship in
  the same MR as I2 or immediately after.
- **I3 — model/resolution API for the compiler front (`v-metaprogramming`, area=`cadenza-ast`/
  `rcdzc`).** The typed surface the rest of the compiler uses: build a `DictSet` from supplied input
  artifacts (bytes → validated flat inline-canonical `Arenas`, keyed by hash), the "resolve then hand
  the compiler a normal `Arenas`" entry point, the `hash_dag`-vs-deref-canonical identity choice (§2.1)
  exposed to callers, and the `MissingDict` diagnostic wording. Hermetic: the builder takes bytes it is
  GIVEN; it never reads a path or fetches. Gate: a reject test for a dict artifact that is itself
  dict-bearing (v1 dicts must be flat) and for a missing import.
- **I4 — invoke-wire integration (`v-agent-harness` leads, `v-metaprogramming` supports).** The
  AST-as-ABI component-invoke primitive accepts dictionaries as ADDITIONAL input artifacts alongside the
  primary AST arg; the host resolves the arg via `decode_with_dicts(arg, dictset)` before type-inference/
  marshalling. This is the FIRST real consumer and the compaction payoff on the hot path. Gate: an
  invoke whose arg is dict-bearing produces the identical result to the same arg encoded inline; a
  missing dict is a clean host-level error, not a panic. Coordinate with the AST-as-ABI marshalling
  work already in flight (v-agent-harness kernel design).

I1 → I2 (+I2b) → I3 are `cadenza-ast`-local and can land back-to-back. I4 waits on the invoke
primitive's generic marshalling landing (v-agent-harness rework-a/b), then composes.

**Sequencing (v-metaprogramming's rec):** start this arc AFTER the in-flight bytes-literal arc (operator
seq 113 `Ast.Bytes`: B2a in-flight, B2b next). It composes cleanly and is strictly additive — a
dictionary entry can be a `Bytes` leaf, and nothing here touches the leaf encoding — so there is no
reason to interleave with the bytes-literal wire. **Split:** `v-syntax` = wire (I1/I2/I2b),
`v-metaprogramming` = model (I3), `v-agent-harness` = invoke-wire consumer/constraint (I4). All three
codec owners are primed and want to be in the build loop.

## 6. Seams / file anchors (where each increment cuts)

Paths below are repo-root-relative under `implementation/seed/crates/` (e.g. `cadenza-ast/src/codec.rs`
= `implementation/seed/crates/cadenza-ast/src/codec.rs`), per §3.

- `cadenza-ast/src/codec.rs` — new header const (near `:159`), `TAG_DICT_REF` (near `:110`), transport
  decode path (parallel to `decode_detailed` `:317`, reusing `read_leaf` and the tree guard), transport
  encode path (parallel to `encode` `:179`), `DecodeError::MissingDict` (`:120`). **Do NOT alter the
  `SCHEMA_HEADER`/`\x00\x01` branch** — that is the frozen identity plane.
- `cadenza-ast/src/ast.rs` (or a small transport module) — the value-only `Hash([u8;32])` (§9.1) and the
  transport-only `DictRef`/`DictSet` types; keep the transport types OUT of the canonical
  `Struct`/`Arenas` used for identity so `encode`/`canon` cannot accidentally emit a ref. `cdz-kernel`
  re-exports / `From`-converts the `cadenza-ast` `Hash` (byte-identical) rather than the reverse.
- `cadenza-ast/src/lib.rs` — re-export the transport surface (`decode_with_dicts`, `encode_with_dicts`,
  `DictSet`).
- `cdz-kernel/src/event_ast.rs` — **read-only invariant:** this path stays on `\x00\x01`. A regression
  test asserts every `Event` still encodes to byte-identical `\x00\x01` (guards A).
- v-agent-harness invoke primitive (I4) — the host resolution seam, alongside the tagged-AST marshalling.

## 7. The gate (what protects it)

1. `cargo test -p cadenza-ast --lib` — the round-trip + hermeticity + refusal tests above; 0 failed.
   Include: dict-free `\x00\x02` ≡ `decode`; ref resolution + graft; `MissingDict`; out-of-range ref;
   canonical `decode` refuses `\x00\x02`; transport is identity-preserving
   (`encode(decode_with_dicts(x,d)) == encode(canonicalize(a))`).
2. **A byte-stability test that `cdzast\x00\x01` output is UNCHANGED** for the existing corpus — the
   frozen-bijection guard for option A. If any `\x00\x01` byte moves, the change is wrong.
3. `cargo xtask gate` — additive fail-set diff only (a dict feature touches no corpus semantics; the
   fail-set must not move). `cargo xtask check` — fmt + clippy `-D warnings` + `codegen --check` clean.
4. Do NOT touch `cdz-runtime`'s frozen `//` comments / `wit/runtime.wit` (`REQUIRED_RUNTIME_HASH`); the
   dict feature is `cadenza-ast`-side and must not perturb the runtime hash.
5. A property/fuzz test: random arenas × random flat dicts round-trip; a dict-bearing artifact NEVER
   decodes via canonical `decode`; a hostile `\x00\x02` (bad ref, cyclic graft, missing hash) is
   classified, never panics (extends the existing decode-totality discipline).

## 8. Deferred extensions (with a chosen default recorded)

- **Automatic dictionary CONSTRUCTION.** v1 honors a supplied dict; it does not CHOOSE what to factor. A
  later increment adds `build_dict(trees) -> (DictSet, refs)` — a heuristic over subtree frequency×size
  (canonical-subtree hashing to find repeats) to synthesize dictionaries and measure the compaction win.
  Default until then: callers supply the dict-set explicitly.
- **Layered dictionaries (dict-imports-dicts).** v1 dicts are FLAT. If a real need appears, make it an
  ADDITIVE v2: allow a dict's bytes to be `\x00\x02`, walk the content-hash graph in `decode_with_dicts`,
  and add a `CyclicDict` guard. Content-addressing makes an honest import graph naturally ACYCLIC — a
  dict's hash is computed over bytes that already contain its imports' hashes, so a dict cannot import
  one whose hash is not yet fixed, i.e. it can only import dicts that already existed when it was
  created (this is a temporal/derivation ordering, NOT a numeric ordering of the hash bytes — hash
  values have no `<` relation to content). A hostile or corrupt `DictSet` could still PRESENT a claimed
  cycle (hash A's bytes name B and B's name A), so the resolver must still detect and refuse it
  (`CyclicDict`) rather than trust the acyclicity. Reserved in the error enum now; not built.
- **Dict identity / GC.** A `DictSet` is caller-owned input; the compiler/host does not persist or GC
  dictionaries in v1. If dicts become a managed store later, GC by content-address reachability from
  live artifacts (out of scope here).
- **Mandatory vs optional in the encoding.** Dict-refs are ALWAYS optional (transport-only). No artifact
  is ever REQUIRED to be dict-bearing; the identity form is always inline.
- **Evolution / versioning (seq-119 "evolution of the dictionary").** Because a dict is content-
  addressed, evolution is FREE and needs no supersession machinery: a "new version" of a dictionary is
  simply a new dict with a new hash. An old artifact pins the OLD dict hash and keeps resolving against
  it; a new artifact imports the NEWER hash. Both coexist in a `DictSet` (a decode supplies whatever
  hashes its artifacts name). A single artifact MAY import multiple dicts (the import section is a list),
  so one body can mix dicts. There is no dict "supersession" or in-place mutation — content-addressing
  makes an updated dict a distinct object, which is exactly the stable-evolution property the operator
  wanted. GC of unreferenced dicts (if dicts ever become a managed store rather than caller-supplied
  input) is by content-address reachability from live artifacts — out of scope for v1.
- **DAG-invariant "canonical" hash.** `hash_dag` (§4.5) is stable for a GIVEN encoding but is NOT
  invariant under a different choice of which subtrees were factored into dicts (two DAGs that deref to
  the same inline tree but factor differently hash differently). If a factor-invariant structural
  identity is ever wanted, that is a v2 refinement (canonicalize the DAG's factoring before hashing);
  v1 does not need it, since the deref-canonical inline hash already provides the factor-invariant
  identity via the transform.

## 9. Open decisions (each with a chosen default — override only with operator sign-off)

1. **Import-section hash type = a value-only `Hash([u8;32])` defined IN `cadenza-ast`** (the bottom
   crate). NOT `cdz-kernel::Hash` — `cadenza-ast` is the bottom crate and `cdz-kernel` depends on IT, so
   referencing `cdz-kernel::Hash` from the wire format would be a dependency-inversion CYCLE (v-syntax's
   ruling, folded in). `cadenza-ast` defines its own `Hash([u8;32])` — value-only: it just stores +
   compares the 32 bytes an import names, with no hashing machinery or extra dep — and `cdz-kernel`
   re-exports / `From`-converts it byte-identically. The VALUE is unchanged (a dict hash is still a
   normal 32-byte content-address); only the type's crate location moves down to where the wire needs
   it. 32-byte width matches the existing content-address.
2. **Import ordering = sorted by hash** (deterministic transport bytes given a ref-set). Default: sort;
   it costs nothing and aids transport de-dup/caching.
3. **`DictSet` key = full content hash.** Default: yes — that is the seq-119 "hashed dictionary imports"
   and gives evolution-by-hash for free (a new dict version is a new hash; old artifacts still resolve
   against the old hash).
4. **Should `encode_with_dicts` be greedy (largest-subtree-first) ref matching?** Default: yes — prefer
   the largest matching subtree so a ref replaces the most inline bytes; a smaller nested match inside a
   larger matched subtree is subsumed. (Purely a compaction heuristic; does not affect correctness since
   any ref set round-trips.)
5. **Primary stored identity of a NEW dict-bearing program = DAG hash-of-hashes vs deref-canonical inline
   hash?** Default per seq-125: the DAG hash (`hash_dag`) is a first-class content address (cheap, no
   deref), and deref-canonical is the always-available transform. The FROZEN inline hash remains the
   identity for everything that exists today (the guarded invariant). This is the one point flagged to
   the operator via the concierge; it does not block the wire or the increments, and flipping the default
   is a one-line change to §2.1 if the operator prefers deref-canonical-as-primary.

---

## 10. Hand-off

Wire half → `v-syntax` (I1/I2/I2b, `cadenza-ast`). Model/resolution → `v-metaprogramming` (I3, the
compiler front surface + diagnostics). First consumer → `v-agent-harness` (I4, invoke wire) — the prime
beneficiary AND the hardest constraint (deps-by-hash-from-CAS invoke model), so it is in the loop from
the start. The PM (`corpus-bugfix`) is asked to point a vertical at I1 first (after the in-flight
bytes-literal arc, per the sequencing note in §5), since I2/I2b/I3 stack on it and I4 waits on the
invoke primitive's generic marshalling. The frozen-bijection guard (§7.2) is the one test that must
never go red — it is the structural proof that option A held.
