# Design — string, list, map, set, and binary pattern matching (rcdzc), one decision-tree engine

**Author:** design pass (compiler). **Audience:** the implementer picking up rcdzc collection/binary
matching, + future me.
**Status:** proposal / handoff — **nothing landed**. Written ahead of implementation in the house style of
`DESIGN-binding-patterns-rcdzc.md` and `DESIGN-effects-rcdzc.md`: it states the
target, the accept/decline boundary, the shared mechanism, the runtime-primitive audit, the pass-by-pass
edits with line anchors, and the subtleties.

The through-line the operator asked for: **match all five holistically on ONE mechanism, and build a
decision tree / trie over the arms instead of a straight if/else chain, so a probe (a byte read, a length
test, a magic-number compare) is done ONCE and shared across every arm that needs it** — the win is
largest exactly for string and binary matching, where today's cascade re-reads the same bytes per arm.

---

## 1. TL;DR — the win, the insight, the pick

**The win.** rcdzc can match a sum and a tuple today. It **cannot** match a string, a list, a map, a set,
or a byte layout — a `(match s ("hello" 1) …)` or `(match bytes ((bin (u32 MAGIC) (bytes rest)) …))`
declines at `infer.rs:698` ("a match on a non-sum scrutinee is a later phase"). Yet the corpus already
pins all of these:

| category | corpus | normative § | gate | new runtime op? |
|---|---|---|---|---|
| **string** literal match | `02-binding-and-control.sexp:706+` (3 cases, **ungated** — seed realizes) | only the generic §*Matching Is Exhaustive* | none | **no** (unroll `bytes-get`) |
| **list** element+rest `(list x .. r)` | `05-compound-types.sexp:996`, `:1014` | core-semantics.md §*A List Is Deconstructed…* | `list-patterns`, `list-pattern-runtime-tail` | **one** — `vec-drop`/`vec-slice` (rest binder only) |
| **map** key-directed `(map (k p) .. r)` | `05-compound-types.sexp:3010` (3 cases) | **MISSING** (corpus cites a § that doesn't exist) | `map-patterns` | **no** (`map-lookup` 34 + `map-remove` 35) |
| **set** pattern | **none** | **none** | — | **no** (`set-contains` 44 + `set-remove` 45) |
| **binary** `(bin …)` (Erlang-style) | `16-binary-matching.sexp` (39 cases) | **MISSING** (only `options/binary-syntax/`) | `binary-matching` | **no** (`bytes-get`/`-slice`/`-len`/`-concat` all exist) |

**The insight (the one that unifies all five).** Every one of these — including the sum/tuple matches rcdzc
already does — is the same shape: **a match arm is a conjunction of PROBES and BINDERS over an opaque
value, and a match is a top-to-bottom disjunction of arms.** A *probe* is a runtime observation that
succeeds or fails (`sum-disc == d`, `vec-len == 2`, `bytes-get(0) == 0x89`, "map has key k"); a *binder*
extracts a sub-value into a name (`sum-payload`, `arr-get i`, `vec-get i`, "read a big-endian u16 at
offset 2", `map-lookup k`). The scrutinee is **never** inspected structurally — only through these ops —
which is exactly the spec's opacity rule for lists (core-semantics.md:141) and the tagless-heap discipline
the runtime already enforces.

Once matching is "an ordered list of probes + binders," the **decision tree / trie** falls out
immediately: two arms that share a leading probe (`(bin (u8 1) (u16 n))` and `(bin (u8 1) (u16 m) (bytes
r))` both read+test byte 0) should test it **once**, then branch. That is a Maranget-style decision tree
(pick a column, partition arms by the test's outcome, recurse) — and for the *sequential* forms (a `bin`
segment walk, a string byte-by-byte compare) it specializes to a **trie keyed by the shared prefix**:
read the discriminating byte/segment once, `LocalSet` it, and cascade equality tests against the *distinct*
continuations, not against every arm. rcdzc's cascade today (`emit_arms`, `select.rs:590`) is the
degenerate right-leaning tree — one column, never shared. This doc generalizes it.

**The pick.** Ship a single **`match.rs` decision-tree compiler** that sits at the Mir→Lir boundary
(select-time), consumes the ordinary-`Hir`/`Mir` pattern arms (no `Pattern` enum — the discipline holds,
`ir.rs:438`), and emits nested `Lir::If/Else/End` (no new IR rung — a decision tree *is* nested `if`s).
**Retrofit the existing sum/tuple match onto it first** (Increment 1 — the gate is the oracle: byte-identical
or better proves the engine), then add one category per increment (string → list → binary → map/set),
each a new probe/binder kind on the same engine. Fixed-width binary and fixed-length lists and string
literals are all **statically-sized**, so they *unroll* into flat Lir — no loop op needed (rcdzc's `Lir` has
no loop; §7). The only genuinely-new runtime primitive across all five is the list **rest-binder tail**
(`vec-drop`) — the exact op the corpus already flags `list-pattern-runtime-tail`.

---

## 2. What the spec and corpus already pin (read before touching code)

The constitution makes the executable corpus the source of truth. Three of the five categories are
**spec-first blocked** — the corpus references normative sentences that do not exist. Step 0 of every
relevant increment is to write them (§11). The surfaces:

### 2.1 String — whole-string literal equality (realized, ungated)
`02-binding-and-control.sexp:706` "matching on string literals":
```lisp
(match "hello" ("hello" 1) ("world" 2) (else 0))   ; => 1
```
Literal patterns match by **value equality**; the doc calls out the compiler idiom directly: *"dispatch on
instruction tags like 'i64.const', 'i64.add' … replacing nested if/= chains with readable match."* That is
the trie's canonical customer. `:716` adds a computed scrutinee (`(String.concat "a" "b")` → `"ab"` arm).
**No** string prefix / head / structural string pattern exists anywhere — only whole-string equality. No
dedicated normative §; it rests on §*Matching Is Exhaustive Or Rejected* (core-semantics.md:111).

### 2.2 List — element patterns with an optional rest (gated)
Full normative §*A List Is Deconstructed By Element Patterns With An Optional Rest* (core-semantics.md:131–141).
Load-bearing sentences: fixed-arity `(list a b)` matches length **exactly** n; `(list x .. rest)` matches
length **≥ n**, binding the rest as *a list of the same type* (so a recursive fn can re-match it, :137); the
matcher observes **only length and elements in order, never a cell/node structure** (:141); an empty-arm +
leading-plus-rest arm is exhaustive (:139). Corpus: `05-compound-types.sexp:996` (static, all scrutinees
inline → decidable at compile time, `list-patterns`) and `:1014` (recursive `sum` over a runtime parameter,
needs a materialized tail → `list-pattern-runtime-tail`, declines "runtime list element-pattern (rest
binder) needs a list-tail primitive" until `vec-drop` lands).

### 2.3 Binary — the `(bin …)` form (gated, the richest surface)
`16-binary-matching.sexp` (39 cases, all `(needs binary-matching)`) + `options/binary-syntax/{README,bin-form}.md`
(adopted default). **One `bin` head, dual direction** — reusing the constructor/pattern duality (`(Some 5)`
builds, `(Some n)` destructures): `(bin <seg>…)` in expression position **constructs** a `Bytes`, in
pattern position **destructures** a `Bytes` scrutinee. Segment grammar (`bin-form.md:28`):

| segment | construct | match |
|---|---|---|
| `(u8 v)`…`(u64 v)` | emit unsigned N-bit, **big-endian default** | bind unsigned N-bit int |
| `(i8 v)`…`(i64 v)` | emit signed N-bit, two's complement | bind signed N-bit int (negative from high bit) |
| `(uNN v le)` / `(iNN v le)` | `le` modifier → little-endian | read little-endian |
| `(bits v k)` | low `k` bits of `v`; **k a compile-time constant** | bind next `k` bits as int |
| `(bytes b)` | splice all of `b` | **final segment only:** bind remaining bytes |
| `(bytes b n)` | splice `b`, length must equal `n` | bind **exactly `n`** bytes; `n` MAY be a name bound by an earlier segment (**dependent size** — "the crown jewel") |

A **literal** in the slot matches by equality (`(bin (u32 0x89504E47) (bytes rest))` = magic-number
dispatch). Rules that shape the compiler:
- **Whole-scrutinee accounting** (`:195`, `:207`): a `bin` pattern matches the **entire** byte sequence —
  leftover bytes are a *non-match* (fall through), which is why a trailing `(bytes rest)` is needed for a
  variable tail. This is the length-test that closes each arm.
- **Static byte-alignment → CDZ0220** (a NEW code, §9): bit-fields must sum to whole bytes; a non-final
  unsized `(bytes b)` is ill-formed; a non-constant `bits` width is ill-formed. All checked at compile time
  because `bits` widths are constants.
- **Runtime fit → trap** (construction side): `(bin (u8 256))`, `(bin (u8 -1))`, `(bin (bits 2 1))` trap
  "binary value does not fit segment" — never truncate/wrap. (Signed `(i8 -1)` → byte 255, no trap.)
- **Exhaustiveness reuses CDZ0210** (`:390`): a `bin`-only arm never covers every byte sequence → needs a
  catch-all, exactly like a sum missing a variant. A bare final `(bin (bytes rest))` matches any Bytes and
  serves as the catch-all.

### 2.4 Map — key-directed patterns (gated, spec-first gap)
`05-compound-types.sexp:3003` (section comment) + 3 cases at `:3010`. `(map (k p) .. rest)` is a
**key-directed lookup**, distinct from the structural tuple/sum/list patterns: it matches when the map
**has** key `k` bound to a value matching `p`, binding `rest` to the map minus the named keys. Lowers to
`Map.lookup` per key + `Map.remove` for the rest. A map's key set is a runtime collection, not a static
shape → **never exhaustive**, needs a catch-all. ⚠ The corpus cites core-semantics.md §*A Map Is Matched By
Key-Directed Patterns* — **that section does not exist**; writing it is a spec-first prerequisite.

### 2.5 Set — no coverage anywhere
Zero corpus cases, zero normative sentences. Fully greenfield. If pursued, the natural analogue of the map
pattern is a membership-directed pattern `(set e .. rest)` — matches when the set **contains** `e`, binds
`rest` to the set minus `e` (lowering `set-contains` 44 + `set-remove` 45, both exist). But this needs
spec-first work AND a design decision the corpus hasn't taken. **Recommend deferring** until a real need;
the engine will accept it as one more probe kind when the spec lands.

---

## 3. The current machinery and exactly where it stops

(Confirmed by reading `ir.rs`, `infer.rs`, `resolve.rs`, `select.rs`, `fold.rs`, `lower.rs`.)

- **Patterns are ordinary `Hir`/`Typed`/`Mir`, no `Pattern` enum** (`ir.rs:438-445`): a `Local` in pattern
  position is a *binding* occurrence, `Wildcard` matches-and-binds-nothing, `(Some x)` is
  `Apply{Ctor,[Local]}`, `(tuple a b)` is `Tuple([Local,Local])`, a literal matches by equality. A pattern
  lowers exactly like the equivalent construction expression. **Keep this discipline** — the new categories
  add no pattern node; `(list a b)` already resolves to `Hir::List`, `(map (k v))` to `Hir::Map`, a string
  literal to `Hir::Str`. A `(bin …)` pattern is the one that needs a resolve-time form (§5.1) because `bin`
  is not otherwise a value head.
- **Match arm type** is `(Hir,Hir)` / `(Typed,Typed)` / `(Mir,Mir)` = (pattern, body). Mir `Match` carries
  `scrut_ty: Ty` + `ty: Ty` (`ir.rs:718`).
- **resolve** (`match_form`, `resolve.rs:1026`; `resolve_arm`, `:1047`) resolves the pattern *as an
  ordinary expression* with binders pre-collected (`collect_binders`, `:1067` — a bare name is a binder
  unless it's `_` or a prelude `Ctor`). No pattern-specific parser. A `(list …)`/`(map …)`/`(set …)` arm
  already resolves to the construction node; a `(bin …)` head is unrecognized → generic decline at `:954`.
- **infer** (`infer_match`, `infer.rs:656`): the two hard gates are
  - the **tuple-scrutinee guard** (`:677`) — a `Ty::Tuple` scrutinee is single-arm destructuring; and
  - the **non-sum decline** (`:696-699`) — after tuple, only `Ty::Sum` is accepted; **anything else
    (String/Bytes/List/Map/Set) declines here.** This is the master gate the new work opens.
  - plus `typed_pattern_simple` (`:705`, def `:822`) declines an inner literal/nested ctor, and
    exhaustiveness (`:712-734`) is **variant-set-only** (`typed_pattern_variant`, `:843`).
- **select** (`emit_match`, `select.rs:556`): the sum path builds a scrutinee local then calls `emit_arms`
  (`:590`) — a **right-leaning `if sum-disc(h)==d … else <rest>` cascade**, binding each arm's payload via
  `bind_pattern`/`bind_payload` (`:629`/`:656`, `arr-get`/`sum-payload` walks). The tuple path (`:565`) is
  single-arm destructure. **This is the code the decision-tree engine generalizes.**
- **Lir has NO loop/block/br** (only `If/Else/End`, `ir.rs`): any iteration is a **fixed compiler-emitted
  helper** (the `utf8-valid`/`putu`/`itoa` precedent, `RT_FIXED_FUNCS=4`, `heap.rs:35`). §7 shows why the
  static-width majority of this work needs **no** loop at all.
- **Runtime op table** (`heap_envelope.rs`, `RT_N_IMPORTS=53`): all of `arr-*`, `vec-empty/len/get/push/
  update/concat` (24–28,41), `bytes-alloc/set/get/len/concat/slice/compact` (13–16,29–31), `sum-*` (10–12),
  `map-lookup/insert/remove/size` (32–36), `set-contains/insert/remove/size` (43–46), `box/get-int`,
  `dup`/`drop`. **The matching side needs no new op except `vec-drop`** (§7).

---

## 4. The shared mechanism — a Matcher IR of probes and binders

Introduce an **internal** (not-an-IR-rung) data structure in a new `match.rs`. It is built from the Mir
pattern arms + `scrut_ty`, and emitted to `Lir`. Nothing about it leaks into `Hir`/`Mir`/`Ty`.

```rust
/// A PROBE is a runtime observation on a sub-value that yields a boolean (match continues / arm fails).
/// Each maps to a small, already-existing runtime op sequence. The scrutinee is opaque — a probe is the
/// ONLY way a pattern observes it.
enum Probe {
    Disc(u32),                       // sum-disc(h) == d           (himport SUM_DISC 11)
    LenEq(usize),                    // vec-len(h) == n   / bytes-len(h) == n   (list fixed-arity; bin whole-consume)
    LenGe(usize),                    // vec-len(h) >= n            (list rest binder: at least n leading)
    ByteEq { off: Cursor, val: u8 }, // bytes-get(h, off) == val   (string trie; bin literal segment)
    IntEq  { seg: SegSpec, val: i64},// read a fixed-width int at a cursor and compare (bin literal int segment)
    KeyPresent(MirConstKey),         // map-lookup(h,k) != NULL / set-contains(h,e)  (map/set pattern head)
    RemainingEq(usize),              // bytes remaining at cursor == n (bin whole-scrutinee close)
    RemainingGe(usize),              // bytes remaining >= n           (bin fixed segment fits)
}

/// A BINDER extracts a sub-value into a fresh local. It runs only on the success path of its probes.
enum Bind {
    Payload,                         // sum-payload(h)             (SUM_PAYLOAD 12)
    ArrGet(usize, Ty),               // arr-get(h,i) unbox by Ty   (tuple/record element)
    VecGet(usize, Ty),               // vec-get(h,i) unbox by Ty   (list element)
    VecDrop(Cursor),                 // list tail after k leading  (NEW op vec-drop — the ONE gap)
    ReadInt(SegSpec),                // read a fixed-width int from a cursor (bin int segment → boxed Int)
    Slice { off: Cursor, len: SizeExpr }, // bytes-slice(h,off,len) (bin (bytes b n) / (bytes rest))
    Lookup(MirConstKey, Ty),         // map-lookup(h,k) value      (map (k p))
    Remove(Vec<MirConstKey>),        // map/set minus named keys   (map/set .. rest)
}
```

`Cursor`/`SizeExpr`/`SegSpec` carry the *binary* specifics — a byte offset that is a compile-time constant
until a dependent-size `(bytes b n)` appears, after which it becomes "constant + a bound local" (§6.4).
`MirConstKey` is a fold-evaluated constant key (map/set patterns require literal keys — a runtime key in a
pattern position declines, since exhaustiveness and the query both need the key known).

A single arm compiles to `Vec<(Probe|Bind step)>` in **left-to-right** order (probes and binds interleave —
a `bin` reads a length byte, binds `n`, then a dependent probe uses `n`). The whole match is the arm list.
The decision-tree compiler (§4.1) turns the arm list into shared control flow.

### 4.1 The decision tree / trie — the operator's ask

Naïve emission is today's cascade: for each arm, emit its probes as a guard, `else` the next arm. The tree
shares work. The algorithm (Maranget, "Compiling Pattern Matching to Good Decision Trees", specialized to
our probe vocabulary):

1. Look at the **first probe position** every remaining arm tests (the "column"). For the sequential forms
   (string, `bin`) this is a byte offset / segment; for sums it is the discriminant; for lists the length.
2. **Partition** the arms by the *value* they require at that position: all arms wanting `disc==0` in one
   group, `disc==1` in another; all string arms wanting `byte(0)=='i'` in one group, `'w'` in another; a
   wildcard/rest arm flows into **every** group (it doesn't constrain this column) and into the default.
3. Emit the probe **once** — `LocalSet` the observed value (the byte, the disc, the length) into a scratch
   local — then a cascade of `== <distinct value>` tests, each descending into the recursively-compiled
   sub-tree for that group. The default edge is the sub-tree of arms that don't constrain this column.
4. Recurse until an arm's probes are exhausted → emit its binders + body.

This is a **trie** when the column is a byte and the groups key on byte value: the discriminating byte is
read once and compared against the *distinct next bytes*, not re-read per arm. For
`("i64.const" …) ("i64.add" …) ("i32.add" …)` the tree reads byte 0 once (all `'i'`), byte 1 once
(`'6'`→two arms, `'3'`→one), and so on — comparisons scale with *distinct prefixes*, not arms×length. For
`(bin (u32 MAGIC_A) …) (bin (u32 MAGIC_B) …) (bin (u16 n) …)` the u32 is read once and compared to the two
magics; the u16 arm branches off after a length check. That is the "not doing the same checks over and
over" the operator wants, and it is largest for exactly the two forms the operator flagged.

**Correctness invariants (do these right or it miscompiles):**
- **First-match order is preserved** (core-semantics.md:115, `02-binding-and-control.sexp:1132`). When two
  arms could match the same value, the earlier arm must win. The partition MUST keep arms in source order
  within a group, and a wildcard/rest arm that flows into a group sits *after* the more-specific arms in
  that group but *before* any later source arm. (Maranget's tree preserves this if the column choice never
  reorders across a wildcard row — start with the simple "always pick the leftmost column" heuristic, which
  is order-safe, before any smarter column selection.)
- **Code duplication is bounded.** A decision tree can duplicate a shared *tail* sub-tree across branches
  (the classic downside). rcdzc should start with **shared-prefix trie + a linear fallback** (share leading
  probes; stop sharing once arms diverge and emit the divergent tails linearly) — this captures the
  string/bin win without risking exponential blowup, and can be generalized later only if a corpus case
  needs it. Log if a match is compiled with the linear fallback so a future optimizer knows where to look.
- **Binders bind on the taken path only** (core-semantics.md:117). A binder emits inside the `then` of its
  probes, so a name is in scope only in its arm's body — matches today's `bind_payload` placement.

### 4.2 Why this is not a new IR rung
A decision tree is **nested `if`** — `Lir::If/Else/End`, which select already emits. The Matcher IR is a
*local* structure inside `match.rs`, built and consumed within one `emit_match` call, exactly as
`emit_arms` today is local control flow, not a persisted node. So: **zero new `Hir`/`Mir`/`Ty`/`Lir`
variants for control flow.** The pattern arms stay ordinary `Mir` (the `ir.rs:438` discipline). The only
IR-adjacent additions are (a) resolve-time recognition of the `(bin …)` pattern/expression head (§5.1) and
(b) `SegSpec`/`Cursor` are internal to `match.rs`.

---

## 5. Pass-by-pass — where each category plugs in

The headline: **most categories touch only `infer.rs` (open the scrutinee-type gate + a pattern arm + the
exhaustiveness rule) and the new `match.rs` (a probe/binder kind).** `bin` additionally needs `resolve.rs`
(a form) and `diag.rs` (CDZ0220). Construction-side `bin` needs `fold.rs`/`select.rs` emission.

### 5.1 `resolve.rs`
- **string / list / map / set patterns:** *no new resolve code* — they already resolve to
  `Hir::Str`/`List`/`Map`/`Set` construction nodes (agent-confirmed). `collect_binders` (`:1067`) already
  recurses `items[1..]`, so `(list a b)`'s `a,b` and `(map (1 v))`'s `v` are collected as binders. The only
  care: a map/set pattern's **key** is not a binder (it's a literal probe) — `collect_binders` already skips
  the head, but a `(map (k v))` entry is `(k v)` where `k` must be treated as a value/probe and `v` as a
  binder; add a small pattern-context rule so `k` in entry-head position is resolved as an expression, not
  collected as a binder. (Mirror the `bin` literal-vs-binder split below.)
- **`bin` form (the one real addition):** add a `Some("bin")` arm to the form dispatcher (near `:882`). It
  parses the segment list into an internal `Vec<Segment>` (shared by BOTH directions — the duality). In
  **expression position** it builds a construction `Hir` (a `Bytes`-producing form; §6.6). In **pattern
  position** (detected because `resolve_arm` resolves the arm's pattern node — thread a `in_pattern` flag,
  or resolve `bin` to a dedicated `Hir::BinPat`/`Hir::BinBuild` pair) it produces the pattern representation
  the matcher reads. A segment slot is a **binder** if it's a bare name, a **literal probe** if it's a
  literal, and a **dependent size** if a `(bytes b n)` names an earlier binder. Per the "no Pattern enum"
  discipline, prefer representing a `bin` pattern as an ordinary `Hir` node whose children are the segment
  binders/literals (a new `Hir::Bin { segs }` used in both value and pattern position, like `Tuple`), rather
  than a bespoke pattern type. **CDZ0220 well-formedness** (byte-alignment, non-final unsized bytes,
  non-const bits width) is a static check here at resolve/infer — it is decidable from the segment list
  alone.

### 5.2 `infer.rs` — open the gate, one arm per category, extend exhaustiveness
- **The master gate** (`:696-699`): replace the flat "only sum" decline with a dispatch on the applied
  scrutinee type — `Ty::Sum` (today), `Ty::Tuple` (today), and now `Ty::String`/`Ty::Bytes`/`Ty::List`/
  `Ty::Map`/`Ty::Set`. Each routes to its arm-checker.
- **`infer_pattern`** (`:746`) gains arms:
  - `Hir::Str(_)` against a `Ty::String` scrutinee — a literal probe, no binder. Type it `String`, unify.
  - `Hir::List(elems)` — a list *element* pattern: unify `expected` with `Ty::List(v)`, infer each element
    sub-pattern against `v`; a trailing rest binder (a `..`-marked tail) binds `Ty::List(v)`. (The rest
    marker needs a surface token — the corpus writes `(list x .. rest)`; reader support for `..` in a list
    form is a small resolve addition.)
  - `Hir::Map(entries)` — a map pattern: unify `expected` with `Ty::Map(k,v)`; each entry `(key p)` requires
    `key : k` (a *constant* — decline a non-const key) and binds `p : v`; a rest binder binds `Ty::Map(k,v)`.
  - `Hir::Bin { segs }` — unify `expected` with `Ty::Bytes`; each integer segment binder binds `Ty::Int`,
    each `(bytes …)` binder binds `Ty::Bytes`; a dependent size `n` must reference an earlier `Int` binder.
- **Exhaustiveness** (`:712-734`) generalizes from "cover every variant" to a per-type rule:
  - sum: cover every variant (today).
  - list: an empty-list arm **and** a leading-plus-rest (or bare-rest) arm ⇒ exhaustive; else CDZ0210
    (core-semantics.md:139).
  - string / bytes / bin / map / set: **never** exhaustive without a top-level wildcard/name catch-all ⇒
    require one, else CDZ0210 (the unbounded-scalar rule already there for `Int`, reused —
    `16-binary-matching.sexp:390`, `05-compound-types.sexp:3013`).

### 5.3 `match.rs` (new) — the engine, called from `select.rs`
`emit_match` (`select.rs:556`) delegates to `match.rs`: build the per-arm probe/binder lists from the Mir
arms + `scrut_ty`, build the decision tree (§4.1), emit `Lir`. The existing sum cascade and tuple
destructure become the `Disc`/`ArrGet` cases of the general engine (Increment 1 proves parity). `bind_payload`/
`bind_pattern` (`select.rs:629`/`:656`) fold into `Bind` emission.

### 5.4 `fold.rs` — compile-time matching (the static-scrutinee corpus cases)
Many corpus cases have a **constant** scrutinee (`(match "hello" …)`, `(match (Bytes.of (list …)) ((bin …)))`,
`05-compound-types.sexp:996` is entirely inline). The fold (`fold.rs:656`) already folds match arms; extend
it to **decide a match at compile time** when the scrutinee folds to a constant Bytes/String/List/Map — read
the constant through the same probe semantics and select the arm. This makes the static corpus cases pass
without *any* runtime op (the const-fold path the rope-bytes and bytes-of cases already ride), and de-risks
the runtime path. Binary **construction** const-folds too: `(bin (u16 258))` → the constant `Bytes.of (list
1 2)` (the corpus asserts exactly this equality).

---

## 6. Per-category emission detail

### 6.1 String literal match — a byte trie, no new op, no loop
A string is a Bytes-backed UTF-8 leaf (`ty.rs:276`; same rep as Bytes). A literal arm `"hello"` is the probe
"scrutinee bytes == the 5 constant bytes of `hello`". Emit as: `bytes-len(h) == 5` then `bytes-get(h,i) ==
byte_i` for each i — **unrolled** (length static → no loop). Across arms, the decision tree reads
`bytes-get(h,0)` **once**, `LocalSet`s it, and cascades `== 'i'` / `== 'w'` …, descending per distinct byte
— the trie. A leftover length mismatch or byte mismatch falls to the next arm / the mandatory `else`.
(Optional later optimization: a fixed `bytes-eq` helper for very long literals; not needed for the corpus,
which is short instruction tags.)

### 6.2 List fixed-arity — no new op, no loop
`(list a b)`: `vec-len(h) == 2` (probe), then `vec-get(h,0)`, `vec-get(h,1)` unboxed by element type
(binders). Static length → unrolled. `(list)` = `vec-len == 0`. The decision tree shares the `vec-len` read
across all list arms and branches on the length value — arms of different fixed lengths are distinct trie
edges; a rest arm is the `LenGe` default edge.

### 6.3 List rest binder — the ONE new runtime op
`(list x .. rest)`: `vec-len(h) >= 1` (probe), `vec-get(h,0)` (bind `x`), and **`rest = vec-drop(h, 1)`** —
the list with the first element removed, *as a list of the same type*. `vec-drop`/`vec-slice` does not exist
(RT_N_IMPORTS stops at 52). This is precisely the `list-pattern-runtime-tail` primitive the corpus flags.
**Runtime request:** append `vec-drop: func(v: u32, k: u32) -> u32` (index 53) — O(log n) on the RRB vector,
consuming its operand under the existing FBIP/dup convention (`emit_consuming_operand`, `select.rs:524`).
Until it lands, the fixed-arity and static-scrutinee list cases (the `list-patterns` gate) ship on the
const-fold + `vec-get` path; the recursive runtime fold (`list-pattern-runtime-tail`) declines cleanly.

### 6.4 Binary `(bin …)` matching — a cursor automaton, unrolled where static
A `bin` pattern is a **left-to-right cursor walk** over the Bytes scrutinee. State = a byte offset (a
compile-time constant until a dependent `(bytes b n)`), plus bound locals. Each segment is a transition:
- **fixed-width int `(uNN n)` / `(iNN n)`** at constant offset `off`, width `w` bytes, endianness, sign:
  first `RemainingGe(off+w)` (probe — else non-match, `16-binary-matching.sexp:273`), then read the `w`
  bytes via `bytes-get(h, off..off+w)` and assemble with shifts/ors (big-endian: `(b0<<((w-1)*8)) | …`;
  `le`: reversed). A **signed** segment sign-extends the top bit; an **unsigned** zero-extends. A **literal**
  slot compares the assembled int to the constant (probe, `:184`/`:262`); a **binder** slot `LocalSet`s it
  (bind, `:119`). All widths ≤ 8 bytes → assembled in an i64 with a fixed unrolled instruction run — **no
  loop, no new op** (`bytes-get` 15 + `I64Shl`/`I64Or`/`I64ExtendI32*`). Advance `off += w`.
- **bit-fields `(bits x k)`**: consume `k` bits from the current bit-cursor within the current byte(s). Since
  every `bits` width is a compile-time constant and the whole `bin` is byte-aligned (CDZ0220), the bit
  offsets are all static → a fixed mask+shift per field (`(byte >> shift) & ((1<<k)-1)`), unrolled. A literal
  bit-field compares; a binder `LocalSet`s (`:162`). Byte-alignment is checked statically at resolve/infer.
- **dependent-size `(bytes body n)`**: `n` is a bound local (bound by an earlier int segment). Probe
  `RemainingGe(off + n)` (dynamic — `off` static, `n` a local; else non-match `:284`), then
  `body = bytes-slice(h, off, n)` (bind — **`bytes-slice` exists**, index 30, O(1)). Advance `off += n` —
  from here the cursor is `constant + Σ dependent locals`, still expressible as an i64 running offset in a
  scratch local. `n == 0` is a valid empty bind (`:295`).
- **final `(bytes rest)`** (unsized, must be last — CDZ0220 else): `rest = bytes-slice(h, off, len(h) - off)`
  (bind). `(:207)`.
- **whole-scrutinee close**: after the last segment, if there is **no** trailing unsized `(bytes rest)`,
  probe `RemainingEq(off) ⇔ bytes-len(h) == off` (the arm matches only if it consumed everything, `:195`,
  `:218`). An empty `(bin)` is `bytes-len(h) == 0` (`:218`/`:229`).

The decision tree shares leading segments across arms: a common `(u32 MAGIC)` prefix reads+tests once; a
common `(u8 tag)` fans out to per-tag sub-trees. This is the trie over the segment sequence — the operator's
"trie on patterns" for the binary case, and the reason a chunked-format parser (PNG/RIFF, `:347`) doesn't
re-read its magic per arm.

### 6.5 Map / set patterns — key/membership-directed, no new op
`(map (k v) .. rest)`: `k` folds to a constant key; probe `map-lookup(h, box k) != NULL` (`MAP_LOOKUP` 34);
bind `v` = that value unboxed by `V` (reuse `emit_map_lookup`'s guard, `select.rs:487`); bind `rest` =
`map-remove(h, box k)` (`MAP_REMOVE` 35) — for multiple named keys, chain removes. Never exhaustive → the
decision tree's default edge is the mandatory catch-all. Set (if spec'd): `set-contains` 44 + `set-remove`
45, identical shape. **No new runtime op** for either.

### 6.6 Binary construction — the dual direction (share the segment table)
`(bin …)` in expression position builds a Bytes. Fixed-width segments: for each, range-check the value
against the segment (trap "binary value does not fit segment" if out of range / negative-into-unsigned /
bit-field too wide — `:436`/`:444`/`:453`), then emit its bytes. The simplest lowering reuses the existing
`bytes-alloc`+`bytes-set` unroll (like `emit_bytes_of`, `select.rs:971`) for the fixed prefix and
`bytes-concat` (29) to splice `(bytes b)` / dependent bodies — O(1) concat, and the rope-backed Bytes
representation (cdz-runtime, `value-heap-runtime.md` §Deferred Materialization Is Permitted Behind The
Observable Bytes) keeps it cheap. Const-folds to the literal `Bytes` when all segment values are
constant (the construction corpus cases assert exact `Bytes.of` equalities, `:40`–`:113`). **One segment
table, two directions** — the `Segment` structure parsed in §5.1 drives both build and match, exactly the
`(Some x)`-builds/`(Some n)`-matches duality the spec leans on.

---

## 7. Runtime-primitive audit (the encouraging part)

| need | op | exists? |
|---|---|---|
| sum disc/payload | `sum-disc` 11 / `sum-payload` 12 | ✅ |
| tuple/list element | `arr-get` 8 / `vec-get` 26 | ✅ |
| list length | `vec-len` 25 | ✅ |
| **list tail (rest binder)** | **`vec-drop`** | ❌ **new — index 53, the only gap** |
| byte read / length | `bytes-get` 15 / `bytes-len` 16 | ✅ |
| byte slice (dependent/rest) | `bytes-slice` 30 | ✅ (O(1)) |
| byte splice (construct) | `bytes-concat` 29 | ✅ (O(1)) |
| map/set probe + bind + rest | `map-lookup` 34/`map-remove` 35, `set-contains` 44/`set-remove` 45 | ✅ |
| int assembly / masks | `I64Shl`/`I64Or`/`I64And`/`bytes-get` | ✅ (unrolled) |

**The whole matching side needs exactly one new runtime op** (`vec-drop`, and only for the list rest
binder), because:
- **fixed-width `bin`, fixed-arity `list`, and string literals are statically sized** → the byte/element
  reads unroll into flat `Lir` (the `emit_bytes_of`/`emit_str` precedent), and rcdzc's loop-free `Lir` is no
  obstacle;
- **variable-length `bin` reads are slices, not loops** → `bytes-slice` (O(1)) already exists;
- **map/set queries are single ops**, not iterations.

The only place a genuine loop would appear is a hypothetical *runtime-length* string equality of unknown
length — but string patterns are whole-**literal** equality (static length) — or a future
`vec-drop`-free recursive list fold. So the fixed-helper escape hatch (a `bytes-eq`/`vec-drop` helper à la
`utf8-valid`) is a backstop, not the main path. Construction-side `bin` uses `bytes-alloc`/`set`/`concat`,
all present.

---

## 8. Exhaustiveness, holistically (one rule table, extends `infer.rs:712`)

| scrutinee | exhaustive iff | else |
|---|---|---|
| sum | every variant covered (or a catch-all) | CDZ0210 |
| tuple | one arm (single shape) | (existing decline for >1 arm) |
| list | empty-arm **and** (leading-plus-rest **or** bare-rest) arm — or a catch-all | CDZ0210 |
| string / bytes / bin | a top-level wildcard/name catch-all present (a bare final `(bin (bytes rest))` counts) | CDZ0210 |
| map / set | a top-level catch-all present (unbounded key set) | CDZ0210 |

This reuses the existing unbounded-scalar reasoning (the `Int`-without-wildcard case) rather than adding a
special case per type — the spec's explicit intent (`16-binary-matching.sexp:29`, `bin-form.md:74`).

## 9. Diagnostics

- **CDZ0210** (exists, `diag.rs:24`) — non-exhaustive, reused for every category (§8).
- **CDZ0201** (exists) — a *shape* mismatch: a list pattern against a non-list scrutinee, a `bin` against a
  non-Bytes scrutinee, a wrong-typed literal segment.
- **CDZ0102** (NOT in rcdzc yet — see `DESIGN-binding-patterns-rcdzc.md` §5.5) — pattern linearity, now
  reachable across list/bin binders (`(bin (u8 n) (u16 n))` repeats `n`). Land it with, or before, this
  work; the recursive non-deduping check there covers these too.
- **CDZ0220** (NEW — `options/diagnostics-schema/coded-span-record.md:69` reserves it; add to `diag.rs`
  `enum Code`) — an ill-formed binary form: bit-fields not closing a byte, a non-final unsized `(bytes b)`,
  a non-constant `bits` width. Decided statically from the segment list at resolve/infer.

## 10. Increment plan (leverage- and dependency-ordered)

0. **Spec-first (blocking for map/set/binary).** Write the missing normative §§: a binary-matching
   capability § (fold `options/binary-syntax/` into RFC-2119 sentences — currently NO capability §); the
   map key-directed § the corpus already cites but doesn't have; if set patterns are pursued, their § +
   corpus (none today). Register the `list-patterns`/`list-pattern-runtime-tail`/`map-patterns` gates in
   `options/realized-capability-set/seed-ignition-set.md` (only `binary-matching`/`sets` are registered).
   String literal patterns and list patterns already have their spec — no step 0.
1. **The engine, retrofit sum+tuple.** Build `match.rs` (probe/bind + decision tree §4) and route the
   *existing* sum cascade and tuple destructure through it. The behavior gate is the oracle: **byte-identical
   or fewer instructions** proves the engine with zero new surface. No new corpus, no risk to the 360-pass
   green.
2. **String literal patterns (byte trie).** Ungated corpus, no new runtime op, no loop, smallest surface,
   highest compiler-idiom value ("dispatch on instruction tags"). Opens the `Ty::String` gate + the byte-trie
   in the engine. `(match s ("i64.const" …) …)` compiles to a shared-prefix trie.
3. **List patterns.** Fixed-arity + `(list)` first (no new op, static, const-fold + `vec-get`). Then the
   rest binder — file the `vec-drop` runtime request (§6.3), ungate `list-pattern-runtime-tail` when it
   lands. `..` rest-token reader support is the one surface addition.
4. **Binary `(bin …)` — the flagship.** The largest surface and the biggest trie payoff. Sub-order:
   fixed-width int segments (match + const-fold construct — the round-trip corpus cases, no new op) → CDZ0220
   well-formedness → literal/magic segments (the trie shines) → bit-fields (static masks) → dependent-size
   `(bytes b n)` + final `(bytes rest)` (`bytes-slice`) → runtime-fit traps on construction. 39 corpus cases
   ungate as coverage climbs.
   - **4a. Runtime NON-FINAL dependent-size segment (deferred sub-slice).** The runtime lowering
     (`lower.rs` `runtime_fn_spine`/the bin-arm walk ~7222) today admits fixed-width int segments plus
     exactly ONE final bytes segment — either unsized `(bytes rest)` OR dependent-size `(bytes payload n)`.
     A **non-final** variable-length segment (e.g. `(bin (u8 n) (bytes body n) (bytes rest))` — a
     dependent-size `body` FOLLOWED by more segments) still `decline`s cleanly at runtime ("a runtime bin
     match with a bit-field or non-final variable-length segment is not yet lowered", lower.rs:7239); the
     SAME shape compiles const-folded (corpus 16-binary §§322/372/1055 use a const `Bytes.of` scrutinee, so
     the const evaluator does the slicing). This is a valid, well-formed form (§6.4: the offset after a
     dependent-size segment is "constant + a bound local"), NOT the ill-formed non-final UNSIZED `(bytes b)`
     that is a permanent CDZ0220. **Implementation:** `Core::BinRestRead`/`BinSizedRead` read
     `bytes-slice(scrutinee, off, …)` with a STATIC `off`; extend them to a DYNAMIC offset expression
     (`static_base + BinIntRead(n)`), and thread the running offset through the segment walk once a
     dependent-size segment appears (§6.4). This is a Core-op signature change across the 5-arm borrow
     discipline (`is_heap_type`/`heap_operand_ownership`/`binding_escapes`/`mark_binder_dups` + BOTH backend
     `expr.rs` arms + `select.rs`) — a focused multi-file slice, hence deferred behind the const path.
     (Characterized by v-patterns during a runtime-vs-const bin-match probe; declines cleanly today so it is
     correct-for-now, a feature gap not a bug.)
   - **4b. GUARDED bin-match arm (deferred sub-slice).** A `(guard (bin <seg>…) <cond>)` arm — a bin pattern
     under a match-arm guard, where the guard cond reads a decoded segment binder (`(guard (bin (u8 n)) (> n
     5))`) — is UNSUPPORTED today, and (unlike 4a) fails at the FRONT of the pipeline: `lower_match`'s bin
     ROUTING checks `head_name(pat) == "bin"` (lower.rs:4076), but a guarded arm's `pat` is `(guard …)`, so
     `head_name` is `"guard"` and the match never routes to `lower_match_bin` — it falls to the SCALAR probe
     path, which lowers the guard's inner `(bin …)` as an EXPRESSION → "unbound name `bin`" (a pattern-position
     poison; scored `todo`, not graded, so it declines cleanly — a feature gap, not a miscompile). The other
     compound-pattern kinds have the SAME latent shape (a guarded `(list …)`/`(map …)` arm), but bin is the one
     witnessed. **Implementation (3 cohesive parts):** (i) ROUTING — peel a `(guard …)` wrapper before the
     `head_name == "bin"`/`"list"`/`"map"` checks at lower.rs:4073–4102 (an `inner_pat` helper, exactly as the
     scalar path already does at 4114); (ii) RESOLVE — add `guard_cond_bin_binds` (resolve.rs, the binary twin
     of `guard_cond_list_binds`/`guard_cond_record_binds`): when `form` is `(guard (bin …) cond)` ascended from
     the cond, resolve a segment binder to a `Resolved::BinField { scrutinee, segs, seg_index }` — exactly what
     Case B gives the arm BODY — inserted as a new Case 6bg after Case 6recg; (iii) `lower_match_bin` — give
     `BinArm::Bin` an optional guard field (peel it in the classifier at lower.rs:7189), and thread it: the
     CONST path (7300–7338) wraps the matched arm's body-return in a guard fold (`core_of` the cond with the
     seg binders resolving to the const scrutinee via Case 6bg; `ConstBool(false)` → continue to the next arm,
     mirroring `lower_match`'s scalar guard fold at 4271–4293), and the RUNTIME path (7250–7286) nests the guard
     INTO the arm predicate (`bytes-len == total & literals-match & <guard>`, guard read via `BinIntRead` — a
     false guard falls through to the next arm's predicate, NOT a trap). The whole-scrutinee borrow/escape arms
     are unaffected (the guard is a boolean read of already-decoded binders, no new heap operand). Miscompile-
     sensitive at the const fold (a wrong guard fold would select the wrong arm), so it is gated behind the
     `todo` witness "a guarded bin-match arm reads its decoded binder and falls through when the guard fails"
     (16-binary), which flips todo→pass when all three parts land. (Characterized by v-patterns Inc-322 while
     probing bin×guard composition coverage; the resolve helper was prototyped + reverted as incomplete-alone.)
5. **Map patterns** (map-lookup+remove, spec-first done in step 0). **Set patterns** only if step 0 took the
   spec decision.

## 11. Subtleties an implementer must get right

- **First-match order is sacred** (§4.1). The trie must not let a shared-prefix optimization reorder a
  wildcard/rest arm ahead of a later specific arm, nor a specific arm behind an earlier catch-all. Start
  with leftmost-column selection (order-safe) before any heuristic.
- **The scrutinee is opaque — probe, never peek.** Every observation is a runtime op; no pattern reads a
  list's internal cells or a Bytes' storage (core-semantics.md:141; the tagless-heap rule). A slice/rope
  Bytes and a flat Bytes must match identically — they do, because `bytes-get`/`bytes-len` are the only
  observations (the rope-backed Bytes representation guarantees this — see the "Bytes rope" section of
  cdz-runtime's `lib.rs`; memory-and-resource-model.md §Sharing Is Not Observable).
- **`bin` whole-scrutinee accounting** — an arm that leaves bytes unconsumed and has no trailing
  `(bytes rest)` does **not** match (a length-close probe, not a trap). Forgetting the close probe silently
  accepts short/long inputs (`16-binary-matching.sexp:195`/`:198` are the tripwires).
- **Signed vs unsigned segment reads differ by one bit** — an `i8` of byte 255 is −1, a `u8` is 255
  (`:140`/`:151`). Sign-extend for `iNN`, zero-extend for `uNN`; the same split governs construction
  (`(i8 -1)`→255, `(u8 -1)`→trap).
- **Dependent-size cursor goes dynamic.** Once a `(bytes b n)` consumes a runtime `n`, the offset is
  `constant + a local`; keep a running-offset scratch local from that point (don't assume constant offsets
  for later segments). A dependent size larger than the remainder is a **non-match** (fall through), not a
  trap (`:284`).
- **Bit-fields are static or ill-formed.** A non-constant `bits` width is CDZ0220 at compile time — never a
  runtime bit-cursor of unknown width. Byte-alignment is a compile-time sum of constant widths.
- **Map/set keys in a pattern must be constant.** A key is a *probe*, and exhaustiveness needs it known;
  a non-const key in `(map (k p))` declines (uncoded, "a map pattern key must be a constant") — do not
  miscompile it into a lookup of a runtime key that changes the coverage story.
- **Const-fold first, runtime path second.** Most corpus cases have constant scrutinees; deciding the match
  in `fold.rs` (§5.4) passes them with zero runtime ops and de-risks each increment before the runtime path
  exists — the same staging the bytes/list/map increments used.
- **No `Pattern` enum, still** (`ir.rs:438`). A `bin` pattern is an ordinary `Hir::Bin { segs }` reused in
  value position; string/list/map patterns are the existing `Str`/`List`/`Map` construction nodes read in
  pattern context. The Matcher IR (§4) is internal to `match.rs`, not an IR rung.

## 12. Ladder placement & related

Sits after the current first-class-types chain and alongside/after `#153` binding patterns (they share the
"patterns are ordinary Hir + a resolve desugar" discipline and the `CDZ0102` linearity code). The engine
(Increment 1) is independent and low-risk; string patterns (Increment 2) are the cheapest real capability
gain; binary (Increment 4) is the flagship and the largest self-hosting unlock (a wasm/section encoder is
`bin` construction; a CBOR/format decoder is `bin` matching). Effects (`#148`) and ANF (`#158`) are
orthogonal — this touches `infer`/`select`/`fold`/a new `match.rs`/`resolve` (for `bin`), not `Lir` control
flow (still just `If`) beyond the one `vec-drop` runtime op.

Related: `spec/semantics/16-binary-matching.sexp` (39 cases, the binary contract);
`options/binary-syntax/{README,bin-form}.md` (the adopted `bin` design + resolved forks);
`spec/capabilities/core-semantics.md` §*A List Is Deconstructed…* (:131) / §*Matching Is Exhaustive…* (:111);
`spec/semantics/05-compound-types.sexp` (:996 list, :3010 map); `spec/semantics/02-binding-and-control.sexp`
(:706 string, :1132 first-match order); `DESIGN-binding-patterns-rcdzc.md` (the sibling pattern doc + CDZ0102);
cdz-runtime's rope-backed Bytes (the "Bytes rope" section of `lib.rs` — the O(1) slice/concat matching
relies on); `options/diagnostics-schema/coded-span-record.md` (:69 CDZ0220).
```
