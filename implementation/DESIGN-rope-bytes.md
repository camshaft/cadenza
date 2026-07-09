# Design — rope-backed Bytes (for the runtime engineer)

**Author:** compiler engineer. **Audience:** runtime engineer (you own `cdz-runtime/`).
**Status:** proposal — pairs with `implementation/RUNTIME-REQUESTS.md` Request 1. Nothing here is landed;
the WIT append needs your sign-off first (§2). I wrote it on neutral ground; if you'd rather it live
under `cdz-runtime/` beside your other `DESIGN-*.md`, move it — your directory, your call.

This is a *how-to*, not a mandate: the representation is yours. But I have the full picture of the
observable contract (it's pinned green in the corpus already) and of the one perf trap that would
silently defeat the whole thing, so this captures both so you don't have to reverse-engineer them.

---

## 1. TL;DR — the win, and the one insight

**The win.** The Cadenza-authored compiler assembles a wasm module by concatenating encoded sections.
With today's `bytes-alloc`/`set`/`get`/`len` that's an O(n²) copy cascade as the module grows. A rope
makes `concat` and `slice` O(1) (no bytes copied) and defers the single flatten to when the bytes are
actually read out — turning the compiler's build-then-emit into O(n) total. This is the highest-leverage
runtime change for self-hosting build time.

**The insight that makes it cheap.** A rope needs **no new `Node` field** and **no change to the free
cascade**. Your node is already `Node { rc, handles: Vec<Handle>, raw: Vec<u8> }` with no kind tag, and
you already treat `handles.len()` as "genuine runtime layout data, not a type tag" (the free cascade's
scan count). A Bytes rope reuses exactly that: **`handles.len()` ∈ {0, 1, 2} selects leaf / slice /
concat.** No discriminant, no new field — fully consistent with the tagless design.

---

## 2. The one hard boundary (same as HANDOFF §3)

This needs two **append-only** WIT ops after the current last index (25 = `map-len`):

```wit
  bytes-concat: func(a: u32, b: u32) -> u32;                  // 26
  bytes-slice:  func(buf: u32, start: u32, len: u32) -> u32;  // 27
```

- Append only — do **not** reorder/rename/remove 0–25 (it mis-links every emitted program).
- Cost to me: a one-time component-envelope re-derivation (I bake 26→`bytes-concat`, 27→`bytes-slice`).
  I do that in lockstep when you confirm the indices — flip Request 1 to 🟢 with the accepted numbers.
- I own the compiler side (emitting these calls, the RC calling convention for them, equality). You own
  the representation behind the ops. We share only the `cdz_runtime.wasm` artifact.

---

## 3. The representation — three node layouts, one existing shape

All three are ordinary `Node`s. Nothing new is stored; `handles.len()` disambiguates. `raw` holds a
little-endian `u32` per number.

| Bytes node | `handles` | `raw` | `bytes-len` is | notes |
|---|---|---|---|---|
| **leaf** | `[]` | the bytes themselves | `raw.len()` | exactly today's Bytes node — unchanged |
| **slice** | `[parent]` | `[off:u32, len:u32]` | decode `len` from `raw` | a view into `parent` |
| **concat** | `[left, right]` | `[len:u32]` | decode `len` from `raw` | `len == bytes-len(left) + bytes-len(right)` |

Why this can't collide with tuples/lists (which also use `handles.len()`): **tagless dispatch.** The
compiler only ever calls `bytes-*` on a value whose static type is `Bytes`, and `arr-*` on a value whose
static type is a tuple/list. The runtime never has to ask "is this handle a Bytes or a 2-tuple?" — the
compiler's static type already answered. So *within* the `bytes-*` ops, `handles.len()` cleanly means
leaf/slice/concat; *within* the `arr-*` ops it means element count. They never cross.

Ownership follows your existing `arr-set` convention (which stores an element handle **without**
dup'ing — the container consumes the caller's reference):
- `bytes-concat(a, b)` **consumes** `a` and `b` — it stores them in the new node's `handles`, no dup.
- `bytes-slice(buf, …)` **consumes** `buf` — stores it in `handles`, no dup.

That's what lets the free cascade Just Work (§5).

---

## 4. The five ops

`bytes-set` is unchanged: the compiler only ever emits it on a freshly `bytes-alloc`'d **leaf** (it
builds a leaf byte-by-byte, or via the const path). It never targets a rope node.

### `bytes-concat(a, b) -> handle` — O(1)
Allocate a node: `handles = [a, b]`, `raw = (len(a) + len(b)).to_le_bytes()`. Consumes `a`, `b`.
Optional identity shortcut: if one side is the empty leaf, return the other (drop the empty one to
respect ownership). Matches the corpus identity cases; not required.

### `bytes-slice(buf, start, len) -> handle` — O(1), total-or-trap
If `start + len > bytes-len(buf)` (checked in `u64` to avoid overflow), **trap** — reuse your
`trap_oob()` (a panic → wasm trap; the gate matches *trap occurred*, not the reason string, so
`trap_oob`'s generic message is fine). Otherwise allocate: `handles = [buf]`,
`raw = [start, len]` little-endian. Consumes `buf`.
Optional: collapse `slice(slice(p, o1, _), o2, l2)` → `slice(p, o1 + o2, l2)` to bound chain depth.

### `bytes-len(h) -> u32` — O(1)
Switch on `handles.len()`: `0` → `raw.len()`; else → decode the stored `len` from `raw`. (Today's
`with_node(h, 0, |n| n.raw.len())` is correct only for the leaf case; this is the one-line generalization.)

### `bytes-get(h, i) -> u32` — see §4.1 (this is the whole ballgame)

---

## 4.1 `bytes-get` and the O(n²) trap — READ THIS

The naive `bytes-get`: walk the tree — leaf → `raw[i]`; slice → recurse `parent` at `off + i`; concat →
`i < len(left) ? left[i] : right[i - len(left)]`. Each call is O(depth).

**The trap:** the compiler's emit step reads the whole thing — `for i in 0..len { bytes-get(rope, i) }`.
If the rope is a right-leaning concat chain (section-by-section build → depth ≈ n), that loop is
**O(n·depth) = O(n²)** — exactly the cost the rope was supposed to kill. A tree-walking `bytes-get`
alone does not deliver the win.

**The fix — flatten on first non-leaf access (memoize).** When `bytes-get` (or equality, or any full
read) hits a non-leaf node, **flatten it to a leaf in place, once**: allocate a `Vec<u8>` of `len`,
fill it by walking the tree, set `node.raw = filled`, `node.handles = []`. Now it's a leaf; every
subsequent `bytes-get` is O(1). One flatten is O(n); the emit loop is O(n) total.

Two things make in-place flatten correct and safe:

1. **It's unobservable.** The flattened leaf has byte-identical content, so no operation can tell the
   difference. The memory model explicitly licenses this: `memory-and-resource-model.md` #Sharing Is Not
   Observable (and its deferral clause — "may defer materializing its contents until an operation
   observes them"). Safe even when the node is **shared** (`rc > 1`): every sharer sees the same bytes
   before and after. Single-threaded, so no race.
2. **Flatten must release the children it drops.** Converting concat→leaf means the node stops owning
   `[left, right]`; converting slice→leaf means it stops owning `[parent]`. So after copying the bytes
   out, **`op_drop` each former child** before clearing `handles`. This is the one place flatten touches
   RC — get it right and everything else is automatic. (Bonus: flattening a slice *is* `Bytes.compact` —
   it materializes the sub-range and releases the parent. So the retention footgun below is partly
   self-healing: any slice that's ever fully read stops pinning its parent.)

Keep the flatten walk **iterative** (explicit stack/worklist), not recursive — same reason your free
cascade is iterative: a deep rope would otherwise overflow the wasm call stack. You're copying into a
pre-sized `Vec<u8>`, so an explicit worklist of `(handle, dst_offset)` is straightforward.

(If you'd rather not mutate on read, the alternative is to keep concat trees balanced so depth is
O(log n) and accept O(n log n) emits — but flatten-on-access is simpler and gives true O(n). Your call;
the observable contract is identical either way.)

---

## 5. RC / free cascade — already correct, zero changes

Because a concat node holds `[left, right]` and a slice holds `[parent]` in `handles`, your existing
iterative `op_drop` reclaims them with **no changes**: it already drains `handles` onto the worklist and
frees transitively. A concat's two children and a slice's parent are reclaimed exactly when the rope's
last reference drops, and a shared child/parent survives until *its* last owner drops — the
`shared_child_survives_until_its_last_owner_drops` test already covers this shape.

**Retention footgun to measure** (`memory-and-resource-model.md` #Retained Storage Is Accounted For What
It Holds Live): a small `slice` of a huge `parent` keeps the whole parent alive (the parent is in the
slice's `handles`, so RC pins it). That's expected and correct. Note it in whatever peak-heap probe you
add; and note flatten (§4.1) is the release valve — `Bytes.compact`, or any full read, converts the
slice to an independent leaf and drops the parent.

The **only** new RC subtlety is the flatten drop in §4.1. Everything else composes for free.

---

## 6. Observable contract (pinned green in `spec/semantics/10-bytes.sexp`)

Your representation, however internal, must reproduce these — they pass today via the compiler's
const-fold path, so the rope is a pure optimization measured against a green gate:

- **`concat(a, b)`** = bytes of `a` then `b`. **Associative by content:** `concat(concat(a,b),c)` and
  `concat(a,concat(b,c))` must yield the same `bytes-get` sequence (a rope may group either way).
  Empty is the identity on both sides.
- **`slice(buf, start, len)`** = the `len` bytes from `start`. Total-or-trap: `start+len > len(buf)`
  traps; `len == 0` is the empty Bytes; `start == len(buf), len == 0` is empty, not a trap.
- **Sharing is not observable:** a sliced/concatenated Bytes is indistinguishable from a freshly-copied
  one by **every** op — `bytes-len`, `bytes-get`, and equality (which the compiler computes by walking
  `bytes-get` on both operands; so two ropes of different shape but identical logical bytes compare
  equal automatically).
- **Acyclic:** a rope node points only to already-existing children → adds no cycle → RC stays complete.

---

## 7. Acceptance tests to add (in your `#[cfg(test)]` style, using `LIVE_NODES`)

Mirror the existing round-trip + RC tests. Concrete ones worth having:

```rust
// round-trip
let a = /* leaf [1,2] */; let b = /* leaf [3,4] */;
let c = op_bytes_concat(a, b);
assert_eq!(op_bytes_len(c), 4);
for (i, v) in [1,2,3,4].iter().enumerate() { assert_eq!(op_bytes_get(c, i as u32), *v); }

// O(1) concat: concatenation allocates ONE node, copies no bytes
let before = live_nodes();
let _ = op_bytes_concat(x, y);           // x,y already built
assert_eq!(live_nodes(), before + 1);    // one concat node, not `len` new leaves

// associativity BY CONTENT (the corpus law)
let l = op_bytes_concat(op_bytes_concat(a, b), c);
let r = op_bytes_concat(a2, op_bytes_concat(b2, c2));  // same bytes, other grouping
for i in 0..op_bytes_len(l) { assert_eq!(op_bytes_get(l, i), op_bytes_get(r, i)); }

// slice across a concat seam
let s = op_bytes_slice(op_bytes_concat(/*[1,2]*/, /*[3,4]*/), 1, 2);  // -> [2,3]

// slice is total-or-trap
#[should_panic] fn slice_oob() { op_bytes_slice(/*len-4*/, 2, 3); }   // 2+3 > 4

// RC: concat's children reclaimed on drop; slice pins parent, freed when slice drops
let before = live_nodes();
let rope = op_bytes_concat(a, b);        // consumes a,b
op_drop(rope);
assert_eq!(live_nodes(), before /* minus the leaves you built */);   // whole rope reclaimed

// the O(n²) guard: a deep concat chain reads out in O(total), and flatten is unobservable
let mut rope = /* leaf [0] */;
for k in 1..1000 { rope = op_bytes_concat(rope, /* leaf [k as u8] */); }  // right-leaning, depth ~1000
let full: Vec<u32> = (0..op_bytes_len(rope)).map(|i| op_bytes_get(rope, i)).collect();  // must be fast
assert_eq!(full.len(), 1000);
// after the first full read, the node is a leaf (flattened): a second pass is O(1)/byte
```

Keep the 16 existing round-trip tests + all RC tests green. Add a peak-heap note if the slice-retention
case matters for your measurements.

---

## 8. Build / verify / report (HANDOFF §7)

```sh
cd .../cdz-runtime && export PATH="$HOME/.cargo/bin:$PATH"
cargo test --release 2>&1 | tail -30                                      # native contract, incl. new tests
cargo component build --release --target wasm32-unknown-unknown 2>&1 | tail -30
shasum -a 256 target/wasm32-unknown-unknown/release/cdz_runtime.wasm      # post this in RUNTIME-REQUESTS
```

Then flip Request 1 to 🟢 with the accepted indices (26/27) and the new sha256. I re-derive the envelope
and re-run the behavior gate; the runtime path for the 10-bytes cases goes green when both sides are in.
**No rush — the const-fold path keeps the corpus green meanwhile, so this is a perf/scale unlock, not a
correctness blocker.**

---

## 9. Open questions for you (answer inline or on the channel)

1. **Flatten-on-access vs. balanced tree** (§4.1) — I recommend flatten; either satisfies the contract.
   If you flatten, confirm you're OK mutating a shared node in place (it's content-preserving, so safe).
2. **`bytes-compact` as index 28?** RUNTIME-REQUESTS Request 2: if flatten (§4.1) is exposed internally,
   an explicit `bytes-compact(buf) -> u32` (force to a flat leaf, equal by content) is nearly free and
   lets me skip an alloc+copy loop on the compiler side. Only if it falls out for free — otherwise I do
   compact with `bytes-alloc` + a `bytes-get`/`set` copy and need no new op.
3. **Slice-of-slice / empty-operand collapse** (§4) — nice-to-have normalizations; skip if they
   complicate the RC bookkeeping.
```
