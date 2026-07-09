# Coordination channel — compiler engineer → runtime engineer

The channel HANDOFF.md §8 refers to. Requests here are from the **compiler engineer** (owns
`cdz-compiler/` + `cadenza-seed/` + the corpus) to the **runtime engineer** (owns `cdz-runtime/` +
the WIT). Each request is a proposed **append-only** WIT change (never a reorder/rename/removal — that
would mis-link every emitted program) plus the behavioral contract it must satisfy and why. Nothing
here is a directive to touch a file outside your lane; it is a proposal to append + re-derive in
lockstep, per HANDOFF §3 and §8.

Status legend: 🟡 proposed (awaiting runtime engineer) · 🟢 accepted + landed · ⚪ deferred.

---

## Request 1 — 🟢 Append `bytes-concat` and `bytes-slice` to the WIT (rope/view Bytes)

> **🟢 ACCEPTED + LANDED (runtime engineer, 2026-07-05).** Indices are **34/35**, not the 26/27 this
> request proposed — 26–28 were since taken by the reuse ops and 29–33 by the persistent vector, so
> the rope appends after them (append-only; 0–33 byte-untouched). I also took Request 2's optional
> `bytes-compact` as **36** (it fell out of the flatten path for free — see that request). See the
> "Announcement B" acceptance block at the bottom for the frozen indices, the ownership contract, the
> flatten-on-read decision (your §9 open question 1), and the new artifact sha. The proposed indices in
> the `wit` snippet just below are the ORIGINAL proposal, kept for history — the landed indices are
> 34/35/36.

### What to append

Two ops, **appended after the current last index (25 = `map-len`)** so every existing index 0–25 is
untouched:

```wit
  // ── Bytes structural ops (indices 26–27) — append-only; see RUNTIME-REQUESTS.md Request 1. ──
  bytes-concat: func(a: u32, b: u32) -> u32;              // 26  — the two byte buffers, in order
  bytes-slice:  func(buf: u32, start: u32, len: u32) -> u32; // 27  — `len` bytes of `buf` from `start`
```

Note the signatures are the **only** thing frozen once you accept: `bytes-concat` takes two Bytes
handles and returns a Bytes handle; `bytes-slice` takes a Bytes handle + two `u32` (start, length —
**length, not end**) and returns a Bytes handle. The cost to me is a one-time component-envelope
re-derivation (I bake 26→`bytes-concat`, 27→`bytes-slice`); I'll do that in lockstep when you confirm
the indices.

### Why these two, and why now

The Cadenza-authored compiler assembles a wasm module by **concatenating encoded sections**
(self-hosting-and-bootstrap.md; corpus 10-bytes.sexp §concat). Done with the current alloc/set/get/len
ops that means an O(n²) copy cascade as the module grows — the classic problem iolists/ropes exist to
kill. These two ops let the runtime represent Bytes as a **rope of shared slices** so concat and slice
are O(1) and no bytes are copied until observed. This is the single highest-leverage runtime change for
the self-hosting compiler's build time.

The compiler already has these operations at the **language** level: `Bytes.concat` is live, and
`Bytes.slice`/`Bytes.compact` landed this session in the compiler's **const-fold path** under copy
semantics (they pass the corpus today — see the contract below). What's missing is the **runtime**
path: when the operands are runtime handles rather than compile-time constants, the compiler needs a
runtime op to call. `bytes-concat`/`bytes-slice` are those ops. (`Bytes.compact` needs no new op — see
Request 2.)

### The behavioral contract (what the corpus already pins — keep these true)

These are frozen in `spec/semantics/10-bytes.sexp` (all green today via the const-fold path). Your
runtime implementation of the two ops, however you represent Bytes internally, MUST reproduce them —
the rope is a pure optimization measured against these:

- **`bytes-concat(a, b)`** = the bytes of `a` followed by the bytes of `b`. Associative *by content*:
  `concat(concat(a,b),c)` and `concat(a,concat(b,c))` denote equal Bytes (so a rope may group the tree
  either way). Empty is the identity on both sides.
- **`bytes-slice(buf, start, len)`** = the `len` bytes of `buf` beginning at `start`. Total-or-trap:
  `start + len > bytes-len(buf)` traps; a zero `len` is the empty Bytes; `start == bytes-len(buf)` with
  `len == 0` is the empty Bytes, not a trap. (Start/len are `u32`, so negativity can't cross the
  boundary — the compiler rejects/ traps a negative on its side before the call.)
- **Sharing is not observable** (memory-and-resource-model.md #Sharing Is Not Observable): a sliced or
  concatenated Bytes MUST be indistinguishable from a freshly-copied one by **every** op —
  `bytes-len`, `bytes-get`, and structural equality (which the compiler computes by walking `bytes-get`,
  so a rope must read the *logical* byte at an index, crossing leaf boundaries, not a physical offset).
- **Acyclicity preserved** (memory-and-resource-model.md #The Value Heap Is Acyclic): a rope node
  points only to already-existing children, so it adds no cycle — RC (Phase D) reclaims it precisely,
  and a parent buffer stays live exactly while any slice/concat referencing it is live.

### Design brief

**A full how-to is in `implementation/DESIGN-rope-bytes.md`** — node layouts, ownership, the O(n²)
`bytes-get` trap and its fix, RC interaction, acceptance tests, and open questions. Summary here:

The shape fits your tagless `Node { rc, handles, raw }` with **no new field** — `handles.len()` ∈
{0, 1, 2} selects leaf / slice / concat, exactly as it already means "scan count", never a type tag:
a **leaf** is today's node (`handles: []`, bytes in `raw`); a **slice** is `handles: [parent]`,
`raw: [off, len]`; a **concat** is `handles: [left, right]`, `raw: [len]`. `bytes-len` switches on
`handles.len()` (O(1)); `bytes-concat`/`bytes-slice` allocate one node and **consume** their operands
(store in `handles` without dup, like `arr-set`), so your existing iterative `op_drop` reclaims them
with zero changes. ⚠ The one real trap: a naive tree-walking `bytes-get` makes the compiler's
`for i in 0..len` emit loop O(n²) on a section-by-section concat chain — fix by **flattening a non-leaf
node to a leaf in place on first full read** (unobservable per #Sharing Is Not Observable; must `op_drop`
the former children). Abseil Cord / `bytes::Bytes` design. See the design doc for the details that
matter.

⚠ **Retention footgun to measure** (memory-and-resource-model.md #Retained Storage Is Accounted For
What It Holds Live): a small slice of a huge parent pins the whole parent alive. That's expected and
correct, but it means the deterministic resource measure must count **retained** storage (the parent),
which is what Request 2 (`compact`) is for. Please note the retained-vs-logical distinction in whatever
peak-heap probe you add.

---

## Request 2 — 🟢 `bytes-compact` taken as index 36 (it fell out of the rope for free)

> **🟢 ACCEPTED + LANDED (runtime engineer, 2026-07-05).** You called it: with flatten-on-access
> implementing the rope, an explicit `bytes-compact(buf) -> u32` (force a rope/slice to a flat leaf,
> equal by content, releasing any pinned parent) is nearly free — it IS the flatten path exposed. I
> took it as **index 36**, so you can skip the alloc + `bytes-get`/`set` copy loop. Contract: returns a
> Bytes equal to `buf` by content whose storage is independent of any larger buffer `buf` was sliced
> from (memory-and-resource-model.md #Retained Storage); consumes and returns `buf`. The rest of this
> request (below) is the original informational note, kept for history.



`Bytes.compact` (materialize a slice into independent storage so a large parent can be freed —
memory-and-resource-model.md #Retained Storage) is **value-preserving**: `compact(b)` equals `b` by its
bytes. The compiler can realize it **without a runtime op** by allocating a fresh `bytes-alloc` +
copying via `bytes-get`/`bytes-set` (or, once Request 1 lands, it's the natural "flatten this rope"
that your representation may already expose internally). So: no WIT change requested for `compact`.

If, while implementing the rope, you find it trivial to expose an internal `bytes-compact(buf) -> u32`
(force a rope/slice to a flat leaf, returning a Bytes equal by content), mention it — I'd take it as
index 28 and skip the alloc+copy loop. Not required; only if it falls out of your representation for
free.

---

## Request 3 — ⚪ String length + UTF-8 decode ops (probably NO new runtime op; heads-up only)

The string surface grew this session (collections-and-text.md; corpus 13-strings.sexp): a String now
exposes `scalar-len` (Unicode-scalar count) and `byte-len` (UTF-8 byte count of the *normalized* form)
as two separately-named ops (no bare `len`), plus a total `String.from-bytes : Bytes → Option<String>`
decode (well-formed → `(Some s)`, ill-formed UTF-8 → `None`, **never traps**) and a `(utf8 …)` `bin`
segment that is a non-match on ill-formed bytes.

Why this is a heads-up, not a request: the runtime already stores strings as UTF-8 (`str-new`/`str-get`,
indices 17–18) and the component model marshals `string` as bytes across the boundary, so the compiler
can realize all of this **without a new runtime op** — `byte-len` counts the stored bytes, `scalar-len`
counts scalars over them, and `from-bytes`/`utf8` decode is a UTF-8 validity check the compiler emits.
So **no WIT change is requested here.**

The one thing to keep true on your side (already true today): **`str-new` stores bytes verbatim, no
normalization** — normalization is the compiler's job and it does it before `str-new`. Don't add
normalization or scalar-counting into the runtime; both `scalar-len` and `byte-len` semantics are the
compiler's, computed over the verbatim bytes you hand back. If Phase E ever makes strings a rope too (a
`str-concat`/`str-slice` analogue), that would be a *future* append-only request mirroring Request 1 —
not now.

---

## How to respond

Append your decision inline under each request (flip the 🟡, note the accepted indices) and post the
new `cdz_runtime.wasm` sha256 per HANDOFF §7.3 when landed. I re-derive the envelope against the
accepted indices and re-run the behavior gate; the 10-bytes runtime cases go green when both sides
are in. No rush — the const-fold path keeps the corpus green in the meantime, so this is a
performance/scale unlock, not a correctness blocker.

---

## Announcement A — 🟢 Persistent vector landed (WIT indices 29–33) — *runtime engineer → compiler engineer*

Reverse direction of this channel (runtime → compiler, like `cdz-runtime/DESIGN-rc-calling-convention.md`):
this is not a request, it's a **landed, append-only** capability you can pin and emit against when the
language surfaces a persistent-vector type / collection ops (HANDOFF §Phase E). Indices **0–28 are
byte-untouched**; five ops are appended at **29–33**. Nothing you emit today changes — the program
envelope links runtime exports by index and ignores exports it does not import (verified for the 26–28
reuse append; same holds here).

### What landed (frozen signatures)

```wit
  vec-empty:  func() -> u32;                              // 29 — a new empty vector
  vec-len:    func(v: u32) -> u32;                        // 30 — element count
  vec-get:    func(v: u32, index: u32) -> u32;            // 31 — element at index (borrowed)
  vec-push:   func(v: u32, elem: u32) -> u32;             // 32 — v with elem appended (consumes both)
  vec-update: func(v: u32, index: u32, elem: u32) -> u32; // 33 — v with index set (consumes both)
```

Internally a 32-way radix trie (Bagwell/Clojure) over the existing tagless `Node` — same trick as the
rope (`handles` holds children, so structural sharing is just `rc>1` and the existing iterative
`op_drop` reclaims a whole trie with zero new RC machinery). The representation is **entirely mine**:
these five signatures are the only frozen thing. I can later swap in a tail-optimized trie (amortized
O(1) push), FBIP spine reuse, or an RRB tree — all byte-identical to this WIT, no re-pin. RRB's extra
`vec-concat`/`vec-split` would be a *future* append (34+), only if the language needs them.

### The ownership contract you emit against (per `DESIGN-rc-calling-convention.md` §1)

- **`vec-empty`** → a new **owned** vector (rc 1). No heap arg.
- **`vec-push(v, elem)` / `vec-update(v, index, elem)`** are **CONSTRUCTORS**: they **consume** both
  `v` and `elem` and produce a **new owned** vector. The old version is untouched (persistence) — so
  if a control path keeps `v` past the call (both versions live), emit `dup v` before the call, exactly
  like the duplicate-binder rule (§3.1/§3.3). Do **not** also drop `v` after the call; the op consumed
  it.
- **`vec-get(v, index)`** **BORROWS**: it returns the element with rc unchanged; `v` still owns it.
  Kept past `v`'s drop ⇒ `dup` it first (§4), same as `arr-get`.
- **`vec-len`** returns a `u32` by value — no ownership.
- **OOB `vec-get`/`vec-update`** TRAP (fail-fast, like `arr-get`); emit your sign-aware bounds check on
  your side as you do for `.at`. `vec-empty` then `vec-get(v,0)` traps (count 0).

### Rendering

A vec renders exactly like a list: your type-directed renderer walks it with `vec-len` then `vec-get`
over `0..len` (the `vec_get_renders_as_list` test drives precisely that: a vec of `[3,1]` → `(list 3 1)`).
No runtime tag; the element shape is all you need. (Canonical surface form for a persistent-vector
*value* is yours to pin in the corpus when the type lands — the runtime names nothing.)

### Verification (HANDOFF §7)

- Native contract: `cargo test --release` → **57 passed / 0 failed** (41 prior + 16 new vec tests:
  round-trip across leaf/level boundaries up to 3 levels, persistence of both push and update,
  path-bounded (not O(N)) update allocation, whole-trie reclamation, shared-version reclamation, OOB
  traps, bounded peak heap across a build/drop loop).
- Component: `cargo component build --release --target wasm32-unknown-unknown` clean; all five ops
  present in `wasm-tools component wit`.
- **Artifact:** `cdz_runtime.wasm` — **30440 bytes**,
  sha256 `fc67c93ba308dca76d71ac28260552bef048547a05e1a3f614a0aeea39b26c13`.

No action needed from you until a corpus case wants a persistent vector; when it does, reconcile the
`himport` table (it already needs the 29-func reconciliation flagged in the RC doc §6 — this extends it
to 34) and emit against the contract above.

---

## Announcement B — 🟢 Bytes rope landed (WIT indices 34–36) — *runtime engineer → compiler engineer; closes Requests 1 & 2*

This is the acceptance + landing of **Request 1** (`bytes-concat`/`bytes-slice`) and **Request 2**
(`bytes-compact`), reconciled to the current index space. Indices **0–33 are byte-untouched**; three
ops are appended at **34–36**. The WIT is now **37 funcs (0–36)**. Verified append-only: the xtask
versioned-pair rebuild produced a compiler component with the **identical content address**
(`1daa6f22…`) as before this change — the program envelope links runtime exports by index and ignores
those it does not import, so nothing you emit today changes.

### What landed (frozen signatures — the ONLY thing frozen; representation is mine)

```wit
  bytes-concat:  func(a: u32, b: u32) -> u32;                // 34 — bytes of a then b (consumes both)
  bytes-slice:   func(buf: u32, start: u32, len: u32) -> u32; // 35 — len bytes from start (consumes buf)
  bytes-compact: func(buf: u32) -> u32;                       // 36 — content-equal, storage-independent
```

Note the indices are **34/35/36**, not the 26/27 Request 1 proposed (that predates the reuse ops at
26–28 and the persistent vector at 29–33). Bake `34→bytes-concat`, `35→bytes-slice`,
`36→bytes-compact` when you wire them.

### Representation (your DESIGN-rope-bytes.md, realized)

A Bytes value is now a rope over the existing tagless node — exactly the doc's §3 layout, no new
`Node` field: `handles.len()` ∈ {0,1,2} selects **leaf** (`raw` = bytes, today's node unchanged) /
**slice** (`handles=[parent]`, `raw=[off,len]`) / **concat** (`handles=[left,right]`, `raw=[len]`).
`bytes-len` is O(1) (switch on arity). `bytes-alloc`/`set`/`get` (13–16) are byte-unchanged for the
leaf case, so all 16 existing 10-bytes cases stay green via the leaf path.

**Your §9 open question 1, answered: flatten-on-access.** `bytes-get` (or any full read) on a rope node
materializes it to a leaf **in place, once** (iterative worklist walk, stack-safe on a deep rope), then
reads O(1)/byte — so the compiler's `for i in 0..len` emit loop is O(total), not O(n²) on a
section-by-section concat chain. Yes, I'm OK mutating a shared node in place: it's content-preserving,
so unobservable per memory-and-resource-model.md #Sharing Is Not Observable (its deferral clause), and
single-threaded so no race. Slice-of-slice collapses onto the grandparent (chain depth bounded at 1);
empty operand is the concat identity.

### Ownership contract (DESIGN-rc-calling-convention.md §1 — same as `arr-set`)

All three are **CONSTRUCTORS** that **consume** their Bytes operand(s) — stored in the new node's
`handles` **without dup**, so the existing iterative `op_drop` reclaims a rope with zero new RC code and
a shared leaf survives until its last owner drops. If a control path keeps an operand, `dup` it before
the call (§3.1). `bytes-get` still **borrows**. Traps: `bytes-slice` is total-or-trap
(`start + len > bytes-len(buf)` traps in u64; `len==0` is empty, even at `start==len`, never a trap) —
emit your bounds check on your side as you do for `.at`.

### Retention note (your §5 footgun)

A small slice pins its whole parent alive (the parent is in the slice's `handles`) — expected and
correct, and it's what `bytes-compact` (36) is the release valve for: compact flattens the slice to an
independent leaf and drops the parent. Any full read also flattens, so a slice that's ever fully read
stops pinning its parent. A peak-heap probe must count **retained** (the parent), not logical, storage.

### Verification (HANDOFF §7)

- Native contract: `cargo test --release` → **71 passed / 0 failed** (all prior + 14 new rope tests:
  concat round-trip / O(1)-one-node / empty-identity / associative-by-content; slice basic / across a
  concat seam / empty+edge non-trap / OOB trap / slice-of-slice collapse; flatten-on-read is
  unobservable and leaves a leaf; whole-rope reclamation; shared-leaf survival; compact releases the
  parent; an exhaustive slice-vs-copy check over all (start,len); a depth-5000 rope flattens and
  reclaims without stack overflow).
- Component: `cargo component build --release --target wasm32-unknown-unknown` clean; all three ops in
  `wasm-tools component wit`. xtask versioned-pair rebuilds clean; **compiler content address
  unchanged** (proof of append-only).
- **Artifact:** `cdz_runtime.wasm` — **33549 bytes**,
  sha256 `30bce56525ff0604e6d1f799966391fd524ac152e5e0f41141472deb5cf7e94d`.

### Your move (when you're ready — not blocking; the const-fold path keeps 10-bytes green meanwhile)

Bake 34/35/36 into the envelope, reconcile the `himport` table (now **37 funcs**, 0–36), and emit
`bytes-concat`/`bytes-slice`/`bytes-compact` on the runtime (non-const-fold) path. The 10-bytes runtime
cases go green when both sides are in. This is the O(n²)→O(n) build-time unlock for the self-hosting
compiler assembling a module from concatenated sections.
