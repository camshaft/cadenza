# Design — CHAMP map + set, and the cursor iteration protocol

**Author:** runtime engineer. **Audience:** compiler engineer (owns `cdz-compiler/`) + future me.
**Status:** proposal — the WIT append (§4) needs a lockstep re-derivation on the compiler side, and
this one, unlike the vector and rope, is **not purely inert**: it *replaces* the map stub, so the
compiler must switch its map build/read call sequence (§8). Nothing is landed yet.

This is the design settled in dialog with the operator over the seam, the hash cache, iteration, and
dispatch. It captures the *reasoning*, not just the shape, because every decision here was a fork we
deliberately took one way over another.

---

## 1. TL;DR — the win, and the three insights

**The win.** A real persistent map/set: `insert`/`lookup`/`remove` in O(log₃₂ N) with structural
sharing between versions, replacing the positional `map-*` **stub** (WIT 21–25) that stores `(k,v)`
pairs verbatim with no hashing, no dedup, and insertion-order layout. A compiler leans on maps and
sets constantly (symbol tables, environments, free-variable sets, visited sets, capability sets), so
this is core self-hosting substrate — the CHAMP is *the* map, the stub was always a placeholder.

**Insight 1 — the seam is byte-level, not serialization-level.** A CHAMP must *hash* a key (to index
the trie) and *compare* keys (to dedup / resolve collisions), but the runtime is **tag-free** — it
holds structure and bytes, never type identity. The resolution: the runtime hashes and compares keys
by a **direct structural walk of the node graph** (`raw` bytes + child `handles`), never by asking the
compiler to serialize a key to canonical bytes and never by calling back into program code. Keys cross
the seam as **plain handles**, exactly as they do to the stub today. No serialization is emitted, no
upcall, no reentrancy. (This replaced an earlier, more expensive proposal where the compiler shipped a
canonical-byte-form `Bytes` blob per operation.)

**Insight 2 — lazy hashing, cached as an internal detail.** The runtime derives a key's hash by that
same structural walk, on demand. *Where* the hash is cached (per-entry vs memoized on every node) is a
**pure internal-representation choice** — the WIT is byte-identical either way, so it is reversible at
any time with zero coordination. v1 recomputes on the rare node split and does not cache; a per-node
hash memo (which would double as an equality fast-path and the substrate for hash-consing) is a named,
measured upgrade in §5.3, deferred until a profile asks for it.

**Insight 3 — cursors, not materialized entry arrays.** Iteration (for rendering, equality, folds)
uses a **cursor**: a bounded descent-stack that yields entries one at a time with no per-iteration array
allocation. This is stream fusion's source side (OCaml `Seq` / Rust `Iterator`): the compiler builds
`map`/`filter`/`fold` as fused combinators over the cursor, collapsing deep transform chains from
O(N·M) to O(N) at compile time. The runtime owes only a non-allocating "pull the next element" — the
cursor is a **stateless/functional** value (advance returns a new cursor), which at `rc==1` is
physically an in-place mutation (FBIP) and at `rc>1` a path-copy — giving forkable iterators
(`peekable`/`tee`/backtracking) for free by the same RC rule as every persistent structure.

**Why it's cheap.** Like the vector and rope, the CHAMP needs **no new `Node` field** — it fits
`Node { rc, handles, raw }` by giving `raw` a bitmap+size meaning and `handles` an entries-then-subnodes
meaning, dispatched (as always) by the compiler's static type, never by a runtime tag. RC and the
iterative free-cascade reclaim a whole trie with **zero new RC code**. This is the third collection
added the same tag-free way.

---

## 2. Why this is correct without a type tag (the load-bearing argument)

Two facts make a purely structural hash/compare coincide with *value* equality:

1. **Keys within one map are homogeneous.** The compiler guarantees one key type per map (a cross-type
   key comparison is a compile-time rejection). So any two keys the runtime ever compares are the same
   static type. A tag-free structural walk cannot confuse a boxed-int key with a string key *because
   two keys in the same map are never an int and a string* — they are both ints, or both strings, or
   both the same tuple shape. The node-level ambiguity (`box-int` of 8 bytes, a string of 8 bytes, and
   a bytes-leaf of 8 bytes are all `handles=[], raw=<8 bytes>`) is therefore **harmless**: whichever
   type it really is, both operands share it, and structural node-equality = value equality.

2. **Every value form has a canonical node representation — with exactly one exception.** Scalars are
   byte-canonical; strings are their UTF-8 bytes; tuples/records/lists (arrays) and sums are structural
   with no slack; the persistent vector's 32-way radix trie is canonical for a given length+contents;
   and the CHAMP is canonical by construction (a given entry set has one node layout regardless of
   insertion order — this is the "C" in CHAMP: **C**ompressed **H**ash-**A**rray **M**apped **P**refix
   tree, *canonical*). The **one exception is the Bytes rope**, which has many node shapes for one
   logical byte string (§4.1 of `DESIGN-rope-bytes.md`).

Consequence for the hash/compare walk:
- For every canonical rep, `hash`/`eq` is a uniform structural walk over `(raw, handles)`, and it is
  automatically **order-independent** for maps/sets used as keys (canonicality means equal maps have
  identical node graphs — no special commutative combine needed).
- For the **one non-canonical rep** (Bytes rope), the seam obligation is one line (§3): a rope key is
  **compacted to a flat leaf before use as a key**. In canonical (flat-leaf) form its structural hash
  and compare are correct. This is the only obligation the seam places on the compiler.

> **The tripwire to write down:** the correctness of tag-free structural key comparison rests on
> "every value form is canonical modulo rope-compaction." A future **RRB** vector (non-canonical
> balancing — flagged as a possible vector upgrade) would **break this** if RRB vectors are ever used
> as keys, unless RRB vectors are also normalized before use as a key. Any new non-canonical
> representation must either be canonical-on-use-as-key or carry a compaction obligation. This is the
> single invariant the whole keyed-collection story depends on.

---

## 3. The seam contract (compiler ↔ runtime)

**Keys and elements cross as plain `u32` handles.** The runtime hashes them (structural walk) and
compares them (structural walk) itself. No canonical-bytes blob, no equality function pointer, no
upcall.

The compiler's **only** obligations:

1. **Homogeneous keys** — already guaranteed (one key type per map/set; cross-type is a compile-time
   rejection). This is what licenses tag-free comparison (§2.1).
2. **Canonical key form** — pass keys in canonical node form. In practice the only non-canonical rep is
   a Bytes rope, so: **emit `bytes-compact` on a Bytes key/element before `map-insert`/`map-lookup`/
   `map-remove`/`set-*`.** Every other value form is already canonical. (A function value has no
   canonical structural form and is rejected as a key by the existing unsatisfied-constraint diagnostic
   — the same reason it cannot be a set element; the runtime never has to enforce it.)
3. **Iteration order is the runtime's, and it is deterministic** (hash order — a fixed function of the
   members, satisfying `deterministic-value-form.md` §"Ordering Of Aggregate Members Is Fixed" for
   *rendering* and *equality*). **But the frozen canonical byte form should sort entries by key/element
   bytes** so the specific hash function never leaks into a frozen contract. That sort is a compiler-
   side serialization detail; the runtime only promises deterministic iteration, which it delivers.

That's the whole seam. It is *lighter* than the vector/rope handoffs in design (no serialization
capability needed) but *heavier* in coordination (§8: the stub is replaced, not extended).

---

## 4. WIT append (indices 37–53; 0–36 byte-untouched)

Append-only after the current last index (36 = `bytes-compact`). Value-type names on the surface even
though map and set share the trie internally.

```wit
  // ── Map (CHAMP, indices 37–45) — the REAL persistent key→value map, replacing the positional
  //    stub (21–25, now vestigial). Keys cross as plain handles; the runtime hashes + structurally
  //    compares them (tag-free, no serialization, no upcall). Value-keyed: there is no slot index.
  map-empty: func() -> u32;                         // 37 — the canonical empty map
  map-insert: func(m: u32, key: u32, val: u32) -> u32; // 38 — m with key↦val (consumes m, key, val)
  map-lookup: func(m: u32, key: u32) -> u32;        // 39 — val, or NULL if absent (borrows; borrows key)
  map-remove: func(m: u32, key: u32) -> u32;        // 40 — m without key (consumes m; borrows key)
  map-size: func(m: u32) -> u32;                    // 41 — entry count (O(1))

  // ── Map cursor (indices 42–45) — non-allocating iteration. `map-iter` borrows m (dups its root to
  //    keep it live). The cursor is a stateless value: `map-iter-next` CONSUMES it and returns the
  //    advanced cursor (in-place when unique, path-copied when shared → forkable). `map-iter-key`
  //    returns NULL when exhausted (a key is never the NULL handle). Projections BORROW.
  map-iter: func(m: u32) -> u32;                    // 42 — cursor at the first entry (or exhausted)
  map-iter-next: func(cur: u32) -> u32;             // 43 — advance (consumes cur)
  map-iter-key: func(cur: u32) -> u32;              // 44 — current key, or NULL if exhausted (borrows)
  map-iter-val: func(cur: u32) -> u32;              // 45 — current value (borrows)

  // ── Set (CHAMP, indices 46–53) — CHAMP minus the value column: entries are 1 handle, not 2. Same
  //    trie core, hashing, comparison, and RC as map. `set-contains` is a TOTAL predicate (no
  //    positional access; sets are unordered). Mirrors the map ops.
  set-empty: func() -> u32;                         // 46
  set-insert: func(s: u32, elem: u32) -> u32;       // 47 — s with elem (consumes s, elem)
  set-contains: func(s: u32, elem: u32) -> bool;    // 48 — total membership (borrows elem)
  set-remove: func(s: u32, elem: u32) -> u32;       // 49 — s without elem (consumes s; borrows elem)
  set-size: func(s: u32) -> u32;                    // 50 — element count (O(1))
  set-iter: func(s: u32) -> u32;                    // 51 — cursor at the first element
  set-iter-next: func(cur: u32) -> u32;             // 52 — advance (consumes cur)
  set-iter-elem: func(cur: u32) -> u32;             // 53 — current element, or NULL if exhausted
```

**Dispatch is static.** The compiler knows the collection type at every use site, so it emits
`map-iter-next` for a map cursor and `set-iter-next` for a set cursor directly — there is **no runtime
branch on a cursor tag** (the impl-vs-dyn distinction: this is the `impl Iterator` path — monomorphized,
no vtable). `map-iter-next` and `set-iter-next` share one Rust trie-walk helper internally; separate
*WIT ops* do not mean separate *implementations*. Keeping them distinct ops (a) reads in value-type
terms, (b) avoids a data-dependent branch in the hot loop, and (c) leaves every internal representation
change invisible across the seam (frozen signature, private body). Array/vector cursors
(`arr-iter`/`vec-iter`) are trivial future appends — they already have non-allocating `len`+`get`, so
their cursor is a one-`u32` index; deferred until the compiler wants a uniform iteration story.

`map-len` (25) and the positional `map-*` ops become **vestigial** (frozen in the WIT, unused once the
compiler switches — §8). `map-size` (41) is the CHAMP entry count; the stub's `map-len` (handles/2) is
meaningless on a CHAMP node.

---

## 5. Representation

### 5.1 The CHAMP node — no new `Node` field

A CHAMP node is an ordinary `Node { rc, handles, raw }`:

- `raw` = `[datamap: u32 LE][nodemap: u32 LE][size: u32 LE]` (12 bytes).
  - `datamap` bit *i* set ⇒ hash-fragment slot *i* holds an **inline entry**.
  - `nodemap` bit *i* set ⇒ hash-fragment slot *i* holds a **subnode**.
  - `size` = total entry count in this subtree (augmented, so `map-size` is O(1); maintained on the
    path-copied nodes during insert/remove — a shared subtree keeps its size). Two equal subtrees have
    equal `size`, so `size` participating in the structural compare is consistent and a cheap
    inequality fast-path.
- `handles` = **data entries first, then subnodes**, each group in ascending bit-position order:
  - map: `[k₀, v₀, k₁, v₁, …, node₀, node₁, …]` (2 handles per entry)
  - set: `[e₀, e₁, …, node₀, node₁, …]` (1 handle per entry)
  - `data_count = popcount(datamap)`, `subnode_count = popcount(nodemap)`.
  - Slot *i*'s entry index = `popcount(datamap & ((1<<i) - 1))`; its subnode index =
    `popcount(nodemap & ((1<<i) - 1))`. Standard CHAMP compaction (no NULL slots).

VEC_BITS analogue: **5 bits per level** (32-way), so a 32-bit hash gives **7 levels** (levels 0–5 use
5 bits, level 6 uses the last 2). Level *L* uses hash bits `[5L, 5L+5)`.

**Tag-free dispatch.** `champ`-family ops read `raw` as bitmaps+size and `handles` as entries+subnodes;
`arr-*`/`bytes-*`/`vec-*` never touch a CHAMP node. As with the rope, the compiler's static type
answers "is this a map?" before any op is emitted; `(handles.len(), raw.len())` is genuine layout the
op-family already knows how to read, never a universal tag.

### 5.2 Collision nodes (real, not "assume 32-bit hashes never collide")

When two distinct keys share all 32 hash bits, they cannot be separated by prefix and land in a
**collision node**, encoded tag-free as: **`datamap == 0 && nodemap == 0 && handles.len() > 0`**. This
is unambiguous:
- normal node → at least one bitmap bit set;
- **empty map** (only valid at a root) → both bitmaps 0 **and** `handles == []`;
- **collision node** → both bitmaps 0 **and** `handles` non-empty (≥2 entries sharing a full hash).

A collision node's `size` = its entry count; it has no subnodes. `lookup`/`insert`/`remove` linear-scan
it (by structural key compare). It only ever appears at max depth (bits exhausted). Helpers
`node_data_count`/`node_subnode_count` handle normal, collision, and empty uniformly so the rest of the
code is oblivious.

### 5.3 Hash caching — deferred (internal, reversible)

v1 does **not** cache key hashes; it recomputes on the rare split (two prefix-colliding keys pushed
deeper). This keeps `raw` fixed at 12 bytes and the code simple. Two upgrades, each a pure internal
change with **zero WIT/coordination cost**, promoted only on profile evidence:
- **Entry-local cache** — store each inline entry's 32-bit hash after the bitmaps in `raw`; saves
  re-hashing the one key pushed down on a split.
- **Per-`Node` hash memo** — a content-derived hash filled on first hash and reused (an unobservable
  memo, like rope flatten). This is the bigger prize: it doubles as an O(1) equality inequality-gate
  *and* is the substrate for global hash-consing (structural dedup → equality becomes pointer
  identity). But it costs ~4 bytes on **every** node (~14% on a minimal wasm32 node), paid by all
  values, most of which are never keys — so it waits until hash-consing or a measured hot path
  justifies it. Same discipline as frame-limited reuse: no global cost for a speculative win.

---

## 6. Hash + structural comparison (iterative, stack-safe)

Both are uniform structural walks over `(raw, handles)`, kept **iterative** (explicit worklist), like
`op_drop` and rope flatten, so a deep key cannot overflow the wasm call stack.

**Hash** — `h(node) = mix(raw_bytes, h(child₀), …, h(childₖ))` for every node type (FNV-1a over `raw`,
then fold in child hashes). Because every rep is canonical (modulo rope-compaction, §3), this is
automatically consistent with structural equality and order-independent for map/set keys. The 32-bit
result indexes the trie.

**Equality** — `eq(a, b)`: same `raw` **and** same `handles.len()` **and** recursively-equal children.
For canonical values this coincides with value equality. Two normalization rules the walk must honor,
both already forced by the seam:
- **Bytes** compare by *logical content*, not tree shape — but §3 guarantees a Bytes key is compacted
  to a flat leaf before it reaches these ops, so at the point of comparison a Bytes key is already a
  leaf and plain `raw` compare is correct. (If a rope ever reaches here, flatten first.)
- **Floats** compare by exact bytes: `-0.0 ≠ 0.0` (distinct bytes, correct), and NaN is one canonical
  form (the compiler canonicalizes NaN before `box-float`, per the existing float-render learning), so
  all NaN keys share bytes. Plain `raw` compare delivers both.

Comparison only fires on a **hash collision** (same trie slot / same full hash), so the common path is
hash-bounded, not compare-bounded.

---

## 7. Cursors — the stateless iterator, and how immutability + RC make it non-allocating

### 7.1 The contract
A cursor is a **stateless/functional** value: `map-iter-next(cur)` returns a *new* cursor `cur'`; the
old `cur` is unchanged if anyone still holds it. Expressed as **two projections** (OCaml `Seq`'s
`Cons` head/tail split) so each op returns a single handle and no per-step pair is allocated:
- `map-iter-key(cur)` / `map-iter-val(cur)` — the head projection (borrow the current entry). Key
  returns **NULL when exhausted** (a key is never the NULL sentinel → unambiguous done-signal, matching
  `map-lookup`'s absent = NULL).
- `map-iter-next(cur)` — the tail projection (consume cur, return the advanced cursor).

### 7.2 Why stateless is free in the loop and powerful in the fork
At `rc == 1` (the ordinary loop: one owner threads the cursor), `map-iter-next` **reuses the cursor's
own cells in place** (the FBIP `reset`/reuse we already ship) — physically a mutation, semantically
pure, **zero steady-state allocation**. At `rc > 1` (the caller `dup`'d the cursor to fork it), advance
**path-copies** the cursor, leaving the other copy frozen at its position — `peekable`/`tee`/
backtracking **for free**, by the identical `rc==1 ⇒ reuse, rc>1 ⇒ copy` rule that gives every
persistent structure its sharing. A cursor is a *linear, borrowing, ephemeral* value — never persisted,
rendered, compared, or sent across the boundary — so a mutable-in-place-when-unique cursor over an
immutable tree is not a contradiction; it is the FBIP identity.

### 7.3 Representation (v1: simple + correct)
A cursor is a `Node`:
- `handles` = the **descent-path frames**, each a **dup'd** reference to a CHAMP node from the root down
  to the current node (`[root, …, deepest]`). Dup'ing the root keeps the whole tree live for the walk;
  dup'ing each frame keeps the RC discipline honest (the cursor **owns** what its `handles` hold, so the
  iterative `op_drop` reclaims the cursor by dropping exactly these path references — no special-casing,
  and no dangling borrow). Bounded: **≤ 7 frames** ever (32-bit hash / 5-bit levels), plus a possible
  collision frame.
- `raw` = `[state: u32][slot₀: u32]…[slotₐ: u32]` — the current data-entry slot at each frame; `state`
  distinguishes *live* from *exhausted*.

`map-iter(m)`: dup `m`'s root, descend to the leftmost data entry (at each node: if it has data
entries, stop at entry 0; else descend into subnode 0; repeat), recording frames+slots. Empty map ⇒
exhausted cursor.

`map-iter-next`: standard iterative in-order successor — advance the deepest frame's slot to the next
data entry; if none, descend into the next subnode (push a dup'd frame); if none, pop (drop that
frame's dup) and retry at the parent's next slot. O(1) amortized (total push/pop = node count), bounded
depth. At `rc==1` this mutates the cursor in place; at `rc>1` it copies the cursor (re-dup the surviving
frames) first.

`map-iter-key(cur)`: deepest frame node, data slot *s* → `handles[2s]` (map) borrowed; NULL if
exhausted. `map-iter-val` → `handles[2s+1]`. `set-iter-elem` → `handles[s]`. The entries are alive
because the frames (hence the deepest node) are held; a borrowed key/val that escapes the loop gets a
compiler-emitted `dup`, exactly like `arr-get`.

**Deferred (internal, no ABI cost):** the dup-per-frame is the simple correct choice; an optimization
is to hold only the root dup + reconstruct the path from recorded slots, trading dups for re-descent.
Left for later, like the hash cache.

---

## 8. Migration — the one non-inert landing (compiler coordination)

The vector and rope landed **purely inert** (the compiler emitted nothing new until it chose to). The
CHAMP **cannot**: the compiler already emits the map stub (`map-alloc`/`map-set`-by-index /
`map-key`/`map-val`-by-index), so realizing the real map means the compiler **switches its map call
sequence**:

| stub (vestigial after switch) | CHAMP replacement |
|---|---|
| `map-alloc(n)` + `map-set(m, i, k, v)` per pair | `map-empty()` then fold `map-insert(m, k, v)` |
| `map-key(m, i)` / `map-val(m, i)` by index | `map-iter` + `map-iter-key`/`-val` + `map-iter-next` |
| `map-len(m)` | `map-size(m)` |

This is **mechanical** (call these ops instead of those), **not a new compiler capability** — which is
exactly what the byte-level-compare seam (Insight 1) bought us: it removed the heavy dependency (the
compiler learning to canonically serialize a runtime value). So the runtime still lands tested and
inert (the new ops exist, nothing calls them until the compiler switches), and the handoff carries a
precise call-migration note rather than "ignore until convenient." The const-fold path and the stub keep
the corpus green in the meantime; the runtime map cases flip green when the compiler switches.

The one thing to settle with the compiler engineer (flagged, not decided here): the map/set **frozen
canonical byte form** should sort by key/element bytes (hash-independent), while the runtime's *cursor*
yields hash order. Both are deterministic; they differ only in whether the sort key is bytes or hash.
The runtime commits to deterministic hash-order iteration; the compiler sorts for the canonical form.

---

## 9. Spec alignment

- `deterministic-value-form.md` §"Ordering Of Aggregate Members Is Fixed" — satisfied: the CHAMP is
  canonical, so equal maps/sets have identical node graphs and identical (hash-order) iteration;
  §3 assigns the byte-form sort to the compiler so no hash detail enters the frozen contract.
- `collections-and-text.md` Maps + Sets — order-independent equality (canonicality), total `contains`
  with no positional access for sets, deterministic iteration. Matches the
  `set-is-a-primitive-collection-not-a-map-of-unit` learning: **Set is CHAMP-minus-the-value-column**,
  the same family, not `Map<T,Unit>`.
- `memory-and-resource-model.md` #The Value Heap Is Acyclic — a CHAMP node points only to
  already-existing children (path-copying), so RC stays complete; #Sharing Is Not Observable — versions
  share subtrees at `rc>1`, indistinguishable from copies.
- `component-abi.md` tag-free contract — a CHAMP is stored, hashed, compared, iterated, and reclaimed
  as structure+bytes whose type is compile-time knowledge the runtime does not hold. The append is
  representation-agnostic (§4 signatures name *what*, never *how*).

Set-algebra (`union`/`intersection`/`difference`) is derivable by the compiler from `insert`+`contains`
+ iteration for v1; efficient structural (recursive-merge) primitives are a **future append**, like
RRB's `vec-concat`.

---

## 10. Acceptance tests (native, `LIVE_NODES` style)

Mirror the vector/rope suites. Map: insert/lookup round-trip; **overwrite dedups** (insert existing key
⇒ size unchanged, value replaced); **collision node** exercised (inject a stub hash that forces two
distinct keys to the same 32 bits → verify both retrievable, size 2, collision-node encoding);
remove (incl. collapsing a collision node back to inline, and removing the last entry → empty);
persistence + structural sharing (v2 = insert(v1,…); v1 unchanged; shared subtrees are `rc>1`, not
copied); whole-map reclamation on drop; shared-version reclamation; bounded peak heap across a
build/drop loop; `map-size` O(1); compound keys (tuple/small-map keys) hash+compare correctly;
order-independent structural equality (two maps built in different insert orders → equal by
`map-iter`); cursor yields every entry exactly once in deterministic order; **cursor fork** (`dup` then
advance one copy; the other stays put); cursor over empty map is immediately exhausted; **cursor
allocates nothing per step at `rc==1`** (peak-heap flat across a full walk). Set: mirror, incl.
`set-contains` total (no trap on absent), dedup, order-independent equality, `union`-by-hand via
insert+iter.

---

## 11. Open questions
1. **Canonical byte-form sort** (§3.3/§8) — confirm the compiler sorts map/set entries by key/element
   bytes for the frozen byte form, with the runtime committing only to deterministic hash-order
   iteration. (My recommendation; needs the compiler engineer's sign-off since it touches the frozen
   `deterministic-value-form` byte layout.)
2. **`set` reuses `map`'s trie internally** — confirm no objection to the shared Rust core behind the
   distinct `set-*` WIT ops (it does not leak across the seam; purely an implementation economy).
3. **Cursor `done` = key/elem returns NULL** vs an explicit `iter-done: func(cur) -> bool` — I fold
   done into the NULL projection (a key/element is never NULL). Say if you'd prefer the explicit bool.
