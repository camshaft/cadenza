# Design — binary-AST dictionary (hashed dict-imports + node-by-index, hermetic transport codec)

**Author:** design agent (`design-ast-dictionary`). **Audience:** `v-syntax` (owns the wire half in
`cadenza-ast`), `v-metaprogramming` (owns the model/resolution side), `v-agent-harness` (the first
consumer — the AST-as-ABI invoke primitive), + future me.
**Status:** design DECIDED — the load-bearing canonicality fork is RESOLVED by the operator (option A,
below). Nothing is landed. This doc pins the shape, the increments (top-to-bottom the way a vertical
lands them), the seams/file anchors, the gate, and the deferred extensions with a chosen default.

The operator floated this across three Slack messages (seq 119–121, verbatim in §1). It is a coherent,
meaningful feature that COLLIDES with a spec-pinned invariant, so it got a design pass with the operator
before any frozen-wire code was cut. `v-syntax` held all dict wire code pending this ruling.

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

- **(A) — CHOSEN. Dict form is NON-CANONICAL TRANSPORT ONLY.** `canon`/`encode` always emit inline;
  the canonical byte form and content-addressing are UNCHANGED. Dict-bearing bytes are an
  accepted-but-non-canonical INPUT that DECODES (resolving dict-refs against a supplied dict-set) and
  then RE-ENCODES to the canonical inline form. The dictionary is purely a wire/transport compaction
  layer (great for the AST-as-ABI invoke wire and at-rest transfer), never the stored/identity form.
  **The frozen bijection is preserved untouched.**
- (B) — rejected. Canonicality becomes dict-relative (identity = `(tree, dict-set)`). More compaction
  reach (the stored/hashed form itself is dict-compressed) but a SPEC CHANGE to the bijection pins and a
  reshape of content-addressing — a program's identity would depend on which dict it imported.
- (C) — rejected. Defer entirely.

**Consequence of A (the spine of this whole design).** There are TWO distinct byte planes:

| plane | header | who emits | who reads | is it the identity? |
|---|---|---|---|---|
| **canonical / inline** | `cdzast\x00\x01` | `codec::encode` (unchanged) | `codec::decode` (unchanged) | **YES** — hashed, content-addressed, stored, `Event::hash` |
| **transport / dict-bearing** | `cdzast\x00\x02` | `codec::encode_with_dict` (NEW) | `codec::decode_with_dicts` (NEW) | **NO** — decodes then re-canonicalizes to the inline plane |

The identity of a program is ALWAYS its inline `cdzast\x00\x01` bytes. A dict-bearing artifact is
resolved+expanded to a normal `Arenas`, and if you want its identity you `encode` that arena — yielding
byte-identical `cdzast\x00\x01` output regardless of how it arrived over the wire. Dict-free bytes stay
`cdzast\x00\x01` **byte-identical to today** — the entire existing corpus and every stored artifact are
untouched.

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
string. Its content hash (the SAME hash used for content-addressing elsewhere — `cdz-kernel`'s `Hash`
over the canonical bytes) is its identity. A dict's importable NODES are the `StructId`s of its own
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
[ import_count:var ] then each import: [ hash:32 ]   # content hashes, in a CANONICAL (sorted) order
[ leaf_count:var ]  then each leaf (identical leaf encoding to v1)
[ struct_count:var ] then each entry: [ tag:1 ]
      TAG_ATOM(0)      → [ leaf_id:var ]                 # unchanged
      TAG_LIST(1)      → [ n:var ][ child_id:var ]*      # unchanged
      TAG_DICT_REF(2)  → [ dict_idx:var ][ node_id:var ] # NEW: node_id into import[dict_idx]'s arena
[ root:var ]
```
The import section is ORDERED canonically (imports sorted by hash) so that a dict-bearing artifact ALSO
has a deterministic byte form GIVEN a fixed ref-set — useful for de-dup/caching of transport artifacts,
though (per A) this is NOT a program-identity claim. `dict_idx` indexes the import list; `node_id`
indexes the referenced dictionary's `structure`. Both are bounds-checked on decode.

### 4.3 Hermetic resolution — `decode_with_dicts`

```rust
/// A resolved set of importable dictionaries, keyed by content hash. Supplied to the decoder as an
/// INPUT ARTIFACT (seq-120) — the decoder makes NO external calls; a hash not present is a hard error.
pub struct DictSet { /* hash -> decoded, inline-canonical Arenas (validated flat: dict-free) */ }

/// Decode a possibly-dict-bearing transport artifact against a supplied DictSet, EXPANDING every
/// dict-ref into the subtree it names, and returning a normal (dict-free) `Arenas`. Total, like decode:
/// never panics, never returns a wrong tree.
pub fn decode_with_dicts(bytes: &[u8], dicts: &DictSet) -> Result<Arenas, DecodeError>;
```
- `cdzast\x00\x01` input → behaves EXACTLY like `decode` (dicts unused); the two never disagree on a
  dict-free artifact.
- `cdzast\x00\x02` input → resolve every import hash against `dicts` (missing → `MissingDict(Hash)`),
  bounds-check each `DictRef` (`dict_idx < import_count`, `node_id < that dict's struct_count`), and
  GRAFT the named subtree in place of the ref, producing a normal `Arenas`. The result is then subject
  to the SAME tree/decode-bomb guard as `decode` (a grafted arena must still be a genuine tree).
- The returned `Arenas` re-encodes via `encode` to canonical `cdzast\x00\x01` — that is the identity.

**The canonical `decode` REFUSES `cdzast\x00\x02`.** Per A, dict-bearing bytes are non-canonical: the
identity-bearing `decode`/`decode_detailed` continue to accept ONLY `cdzast\x00\x01` and return
`BadHeader` on `\x00\x02` (refuse-on-mismatch, `ast-encoding.md` §The Encoding Is Versioned). Only the
explicitly-transport `decode_with_dicts` accepts `\x00\x02`. This is the structural guarantee that a
dict artifact can never be mistaken for an identity artifact.

### 4.4 The transport encoder — `encode_with_dict` (honor-supplied-dict)

```rust
/// Encode `arenas` as a transport artifact that REFERENCES the supplied dictionaries: any subtree of
/// `arenas` that is structurally equal to an importable node of some dict in `dicts` MAY be emitted as
/// a DictRef instead of inline. v1 emits a ref for an EXACT subtree match against a caller-SUPPLIED
/// dict-set; it does NOT choose which subtrees to factor into a dictionary (that is dict CONSTRUCTION,
/// deferred — §8). Round-trips: decode_with_dicts(encode_with_dict(a, d), d) == canonicalize(a).
pub fn encode_with_dict(arenas: &Arenas, dicts: &DictSet) -> Vec<u8>;
```
**Decided: v1 = decode/resolve + honor-supplied-dict.** v1 delivers the transport codec and an encoder
that emits refs against a dict-set the CALLER supplies. Automatic dictionary CONSTRUCTION (scanning a
corpus, choosing high-frequency/large repeated subtrees to factor into a dict, emitting `(dict, refs)`)
is a separate, later increment (§8). This keeps v1 small and PROVABLE — the round-trip and hermeticity
properties are the whole correctness story and are testable without a heuristic builder.

### 4.5 The decode-error surface

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
  `IdOutOfRange`; canonical `decode` REFUSES `\x00\x02` (`BadHeader`). This is the load-bearing slice.
- **I2 — transport encoder honoring a supplied dict (`v-syntax`).** Add `encode_with_dict`: emit a
  `DictRef` for a subtree that EXACTLY matches an importable node of a supplied dict; else inline.
  Emit imports in canonical (hash-sorted) order. Gate: the ROUND-TRIP identity
  `decode_with_dicts(encode_with_dict(a, d), d) == canonicalize(a)` for a matrix of trees × dict-sets
  (empty dict, matching dict, superset dict), AND `encode(decode_with_dicts(...)) == encode(a)`
  (transport is identity-preserving). A fuzz/property test over random arenas + random dicts.
- **I3 — model/resolution API for the compiler front (`v-metaprogramming`, area=`cadenza-ast`/
  `rcdzc`).** The typed surface the rest of the compiler uses: build a `DictSet` from supplied input
  artifacts (bytes → validated flat inline-canonical `Arenas`, keyed by hash), the "resolve then hand
  the compiler a normal `Arenas`" entry point, and the `MissingDict` diagnostic wording. Hermetic: the
  builder takes bytes it is GIVEN; it never reads a path or fetches. Gate: a reject test for a dict
  artifact that is itself dict-bearing (v1 dicts must be flat) and for a missing import.
- **I4 — invoke-wire integration (`v-agent-harness` leads, `v-metaprogramming` supports).** The
  AST-as-ABI component-invoke primitive accepts dictionaries as ADDITIONAL input artifacts alongside the
  primary AST arg; the host resolves the arg via `decode_with_dicts(arg, dictset)` before type-inference/
  marshalling. This is the FIRST real consumer and the compaction payoff on the hot path. Gate: an
  invoke whose arg is dict-bearing produces the identical result to the same arg encoded inline; a
  missing dict is a clean host-level error, not a panic. Coordinate with the AST-as-ABI marshalling
  work already in flight (v-agent-harness kernel design).

I1 → I2 → I3 are `cadenza-ast`-local and can land back-to-back. I4 waits on the invoke primitive's
generic marshalling landing (v-agent-harness rework-a/b), then composes.

## 6. Seams / file anchors (where each increment cuts)

- `cadenza-ast/src/codec.rs` — new header const (near `:159`), `TAG_DICT_REF` (near `:110`), transport
  decode path (parallel to `decode_detailed` `:317`, reusing `read_leaf` and the tree guard), transport
  encode path (parallel to `encode` `:179`), `DecodeError::MissingDict` (`:120`). **Do NOT alter the
  `SCHEMA_HEADER`/`\x00\x01` branch** — that is the frozen identity plane.
- `cadenza-ast/src/ast.rs` — the transport-only `DictRef`/`DictSet` types (a transport module; keep them
  OUT of the canonical `Struct`/`Arenas` used for identity so `encode`/`canon` cannot accidentally emit
  a ref).
- `cadenza-ast/src/lib.rs` — re-export the transport surface (`decode_with_dicts`, `encode_with_dict`,
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
  ADDITIVE v2: allow a dict's bytes to be `\x00\x02`, walk the content-hash DAG in `decode_with_dicts`,
  and add a `CyclicDict` guard (content-addressing makes the import graph naturally a DAG — a hash can
  only reference pre-existing lower hashes — but the resolver still refuses a claimed cycle). Reserved
  in the error enum now; not built.
- **Dict identity / GC.** A `DictSet` is caller-owned input; the compiler/host does not persist or GC
  dictionaries in v1. If dicts become a managed store later, GC by content-address reachability from
  live artifacts (out of scope here).
- **Mandatory vs optional in the encoding.** Dict-refs are ALWAYS optional (transport-only). No artifact
  is ever REQUIRED to be dict-bearing; the identity form is always inline.

## 9. Open decisions (each with a chosen default — override only with operator sign-off)

1. **Hash width in the import section = 32 bytes** (matches `cdz-kernel`'s `Hash`). Default: reuse the
   existing content-hash type verbatim so a dict hash IS a normal content-address.
2. **Import ordering = sorted by hash** (deterministic transport bytes given a ref-set). Default: sort;
   it costs nothing and aids transport de-dup/caching.
3. **`DictSet` key = full content hash.** Default: yes — that is the seq-119 "hashed dictionary imports"
   and gives evolution-by-hash for free (a new dict version is a new hash; old artifacts still resolve
   against the old hash).
4. **Should `encode_with_dict` be greedy (largest-subtree-first) ref matching?** Default: yes — prefer
   the largest matching subtree so a ref replaces the most inline bytes; a smaller nested match inside a
   larger matched subtree is subsumed. (Purely a compaction heuristic; does not affect correctness since
   any ref set round-trips.)

---

## 10. Hand-off

Wire half → `v-syntax` (I1/I2, `cadenza-ast`). Model/resolution → `v-metaprogramming` (I3, the compiler
front surface + diagnostics). First consumer → `v-agent-harness` (I4, invoke wire). The PM (`corpus-
bugfix`) is asked to stand up / point a vertical at I1 first, since I2/I3 stack on it and I4 waits on
the invoke primitive's generic marshalling. The frozen-bijection guard (§7.2) is the one test that must
never go red — it is the structural proof that option A held.
