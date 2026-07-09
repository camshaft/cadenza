# The Perceus dup/drop calling convention (runtime → compiler)

Status: **contract / coordination** (runtime engineer → compiler engineer). The RC *mechanism* is
built, live, and proven natively (`op_dup`/`op_drop`, 28 tests incl. shared-subtree survival, 200k
deep-spine cascade, bounded peak heap). It is **dormant**: the compiler emits no `dup`/`drop` calls
(`himport::{DUP,DROP}` have zero call sites), so shipped programs are byte-unchanged and leak-by-
process-exit today. This document specifies **where the compiler must emit `dup`/`drop`** to make
reclamation live and correct. Getting the *insertion* wrong is a use-after-free (free too early) or a
leak (never free); the mechanism can't save an incorrect call schedule.

Grounded in the actual WIT (`wit/runtime.wit`) and the actual runtime semantics (`src/lib.rs`), and
in Reinking, Xie, de Moura & Leijen, *Perceus: Garbage Free Reference Counting with Reuse* (PLDI
2021), which this follows.

---

## 0. What is reference-counted

Only **heap handles** (`Kind::Heap` — arrays/tuples/records/lists, sums, maps, and any boxed leaf
that lives on the heap: boxed int/bool/float, bytes, string leaves). A handle is one reference to a
node; the node's `rc` counts how many references exist.

**Not** reference-counted (no dup/drop, ever):
- Unboxed scalars flowing as wasm values — `get-int`/`get-bool`/`get-float` return `i64`/`bool`/`f64`
  **by value**. Once you've called `get-int`, the `i64` on the stack owns nothing.
- `str-get` returns a **fresh component-owned `string`** (the component model copies it across the
  boundary). It is not a handle; do not drop it.
- Compile-time constants folded to baked text (the `CVal` path) never touch the heap.

The rest of this document is entirely about heap handles.

---

## 1. The ownership ABI — what each WIT function does to refcounts

This is the load-bearing table. It is **fixed by the runtime's implementation**; the compiler must
emit against it exactly.

### Constructors — **CONSUME** the handles you pass in

| function | consumes | produces |
|---|---|---|
| `box-int` / `box-bool` / `box-float` | (a scalar, not a handle) | a **new owned** handle (rc=1) |
| `bytes-alloc` / `str-new` | (raw data) | a **new owned** handle (rc=1) |
| `arr-alloc(n)` | nothing | a **new owned** array (rc=1), `n` NULL slots |
| `arr-set(arr, i, elem)` | **`elem`** (the element reference is moved into the array) | returns `arr` as a **borrowed alias** — same reference, rc unchanged |
| `sum-new(disc, payload)` | **`payload`** | a **new owned** sum (rc=1) holding the payload |
| `map-alloc(n)` | nothing | a **new owned** map (rc=1), `n` NULL pairs |
| `map-set(m, i, k, v)` | **`k` and `v`** | returns `m` as a **borrowed alias** — rc unchanged |

Two consequences the compiler must respect:

1. **`elem`/`payload`/`k`/`v` ownership transfers into the container.** After `arr-set(arr, i, x)`,
   the array owns `x`; the caller must **not** also drop `x` (that would double-free), and must
   **not** reuse `x` as if it still owned it (see §3, duplicate binder).
2. **`arr-set`/`map-set` return a *borrowed alias* of the container, not a new owner.** The
   returned handle is the same reference with rc unchanged. Discarding it with the wasm `drop`
   opcode (as `gen_runtime_ctor` does today at `codegen.rs:3683`) is correct and is **not** an RC
   `drop` — do not turn it into a `himport::DROP`. The container's single owner remains the local
   that holds it.

### Accessors — **BORROW**; the parent keeps ownership

| function | ownership of the result |
|---|---|
| `arr-get(arr, i)` | **borrowed** — returns `handles[i]` with rc unchanged. `arr` still owns it. |
| `sum-payload(s)` | **borrowed** — returns the payload with rc unchanged. `s` still owns it. |
| `map-key(m,i)` / `map-val(m,i)` | **borrowed** — rc unchanged. `m` still owns it. |
| `arr-len` / `sum-disc` / `bytes-len` / `map-len` | return a `u32` by value — no ownership. |
| `get-int` / `get-bool` / `get-float` | return a scalar by value — no ownership. |
| `str-get` | returns a fresh component-owned string — no handle. |

The rule that falls out: **a borrowed handle is only valid as long as its parent is.** If you
extract a child and need it to outlive the parent (return it, store it in another container, bind it
past the parent's drop), you must `dup` the child **before** the parent is dropped. This is §4.

### RC ops

- `dup(h)` — `h.rc += 1`. A new owner now exists. Null is a no-op.
- `drop(h)` — `h.rc -= 1`; at 0, free `h` and iteratively drop the children it owns. Null is a
  no-op. The cascade is stack-safe (iterative worklist) and reclaims shared subtrees only when their
  last owner drops.

---

## 2. The discipline in one sentence

> **Every heap value is created with exactly one owner (rc=1); every owner must, along every control
> path, either transfer its reference (consume it into a constructor / pass it to a callee / return
> it) exactly once, or `drop` it exactly once — and any value used by more than one owner gets a
> `dup` per extra owner.**

"Owner" = a binding, a temporary, a function parameter, or the function's return slot. The compiler's
job is to insert `dup`/`drop` so that this balance holds on **every** path (both `if` arms, every
`match` arm). Perceus proves that for an acyclic immutable heap this yields *garbage-free* execution:
a value is freed at the exact point its last owner is done.

---

## 3. Insertion rules per IR construct

Baseline convention (simplest correct form; §5 gives the standard optimizations):

- **Owned parameters.** A function receives each heap parameter as **owned**: the callee is
  responsible for consuming or dropping it. (Borrowed params are a §5 optimization.)
- **Owned return.** A function returns an **owned** handle: the caller must consume or drop it.

### 3.1 Variable use — dup on all-but-last, drop if dead

Let a heap binding `x` be owned in a scope, and count its **dynamic occurrences** along a path:

- **0 occurrences (dead binding):** emit `drop x` at the end of the scope. The binding owns a
  reference nobody consumes; release it.
- **1 occurrence:** that occurrence **consumes** the owned reference. Emit nothing extra.
- **n ≥ 2 occurrences:** emit `dup x` before each of the first n−1 occurrences; the n-th consumes
  the original. (Each dup creates the extra owner that occurrence will consume.)

Equivalent, mechanically simpler to emit and easier to prove, at the cost of one extra dup+drop pair:
`dup x` before **every** occurrence, and `drop x` once at end of scope. Optimize to the above later.

### 3.2 `let x = e in body`

`e` produces an owned handle bound to `x`. Apply §3.1 to `x`'s occurrences in `body`. If `x` is a
scalar/string (not a handle), no RC.

### 3.3 `(tuple … x … x …)` and every constructor with repeated / multi-owner args

`(tuple x x)` stores **two** references to `x`, both owned by the tuple. Emit `dup x` for the first
slot; the second slot consumes the original:

```
dup x                 ; x.rc: 1 -> 2   (one extra owner for slot 0)
arr-set(arr, 0, x)    ; slot 0 owns one reference
arr-set(arr, 1, x)    ; slot 1 consumes the original
```

Now the tuple owns two references to `x`; dropping the tuple drops `x` twice (rc 2→0), reclaiming it
once. This is exactly `shared_child_survives_until_its_last_owner_drops` in the test suite. **This is
the open dup-binder question from the build-run notes, answered: it is a `dup`, not an error.**

### 3.4 `if c then t else e` — **branch balancing**

The critical rule. Let `V` = the set of owned heap values live at the `if`. Each arm must leave the
**same** ownership state: every value in `V` must be consumed exactly once **or** dropped exactly
once **within each arm**. Concretely, for each `x ∈ V`:

- If `x` is consumed in `t` but not in `e`: emit `drop x` in `e` (and vice-versa).
- If `x` is consumed in both, or dropped in both: balanced already.

The result handles of `t` and `e` are both owned and become the owned result of the `if` — no action
needed there. Example — `(if c xs ys)` returning one of two owned lists, both live at the `if`:

```
then arm: (result = xs)   drop ys      ; ys not returned here -> release it
else arm: (result = ys)   drop xs      ; xs not returned here -> release it
```

Without balancing, one arm leaks and the other double-frees.

### 3.5 `match s { … }` — dup extracted fields, then drop the scrutinee

`s` (the scrutinee) is owned. `sum-disc s` (by value) selects the arm; `sum-payload s` **borrows** the
payload. For the taken arm binding `x = sum-payload(s)`:

- If the arm's body keeps `x` (returns it, stores it in a constructor, binds it past `s`): emit
  `dup x` **before** dropping `s`, then `drop s`. The dup makes `x` a real owner; dropping `s`
  frees only the sum node (payload rc went 1→(via dup)2→(via drop-cascade)1), leaving `x` valid.
- If the arm's body does **not** keep `x` (e.g. returns a constant): no dup; `drop s` reclaims the
  whole sum including the payload.

**Ordering is mandatory: `dup` the kept field(s) BEFORE `drop s`.** Dropping first could free the
payload the borrow points at (use-after-free). This is §4 restated for sums; the same holds for
tuple/record projection kept past the parent (`dup (arr-get t i)` before `drop t`) and map lookup
results kept past the map.

### 3.6 Function application `(f a b …)`

Under owned parameters: each argument reference is **consumed** by the call (moved to the callee).
Apply §3.1 to argument variables (dup if the arg variable is used again after the call). The result
is an owned handle the caller now holds — subject to §3.1 like any other owned value.

### 3.7 Lambda / closure capture

A captured heap variable becomes owned by the closure at capture: `dup` it into the closure
environment at creation (an extra owner), and the closure's `drop` releases the environment. (Only
relevant once closures allocate a heap environment; scalar-only or non-escaping lambdas need
nothing.)

### 3.8 The program result

`run` returns the top-level owned result handle. The render harness walks it via **borrowing**
accessors (it never consumes it), then must `drop` it once when rendering is complete — that single
drop cascades and reclaims the entire result graph. (In a run-once process this is not needed for
correctness, but specifying it keeps the convention total and lets the peak-heap probe hit baseline.)

---

## 4. The one ordering invariant, isolated

Because accessors borrow, the single ordering rule that prevents every use-after-free is:

> **Before you `drop` a parent, `dup` every child of it you intend to keep.**

"Keep" = the child (or a further descendant reached through it) outlives the parent: it is returned,
stored into another live container, or bound to a name used after the parent's drop. If you keep
nothing, just drop the parent. Never drop the parent while a bare borrowed child handle is still
going to be used.

---

## 5. Optimizations (defer; correctness first)

Take these only after the baseline above passes the gate; each is standard Perceus and preserves the
§2 balance:

- **Borrowed parameters.** A parameter only *inspected* (read via accessors, never stored/returned)
  can be passed **borrowed**: caller keeps ownership, callee emits no drop. Removes dup/drop traffic
  for read-only args. Needs a per-parameter borrow analysis.
- **dup/drop fusion & cancellation.** `dup x; drop x` cancels. `drop x; dup x` on the same path is a
  no-op. A `dup` immediately consumed can be elided (the "own the last use" form of §3.1).
- **Reuse (FBIP) — the high-value one. Runtime side LANDED; emission contract in §8.** When an
  `rc==1` value is consumed and a value is built in the same breath (record update, `List.map`, a
  functional cons/tree rebuild), hand the dying node's shell straight to the new allocation instead
  of free→malloc. The runtime now ships the three ops this needs — `reset`, `arr-alloc-reuse`,
  `sum-new-reuse` (WIT indices 26–28, appended; existing 0–25 byte-preserved). This is the last WIT
  touch the reuse story needs. Correctness of the baseline convention (§1–§4) is a prerequisite:
  emit reuse only once plain dup/drop is green end-to-end.

---

## 6. Prerequisite the compiler must fix first (coordination)

The compiler's `himport` indices are **desynced** from the frozen WIT and MUST be reconciled before
any `dup`/`drop` call is emitted, or the calls will mis-link to the wrong runtime function:

- WIT (frozen/append-only, authoritative): `… str-new=17, str-get=18, dup=19, drop=20,
  map-alloc=21 … map-len=25, reset=26, arr-alloc-reuse=27, sum-new-reuse=28` (**29 funcs**).
- Compiler `himport` (`codegen.rs:4266`+): omits `str-new`/`str-get`, has `dup=17, drop=18,
  map-alloc=19 … map-len=23`, `RT_N_IMPORTS=24`.

The envelope lowers 24 of the (now) 29 funcs. Reconcile the index table (and lower all 29, or
explicitly document those it skips) so `himport::DUP`/`himport::DROP` resolve to WIT 19/20 and the
reuse ops to 26/27/28. **Note (verified 2026-07-05): appending exports 26–28 did NOT break the
existing gate — 422 pass / 0 fail against the 29-func artifact.** The program envelope links the
runtime's exports by index and simply ignores exports it does not import, so the reuse ops are inert
until the compiler chooses to emit them. This is the compiler engineer's file; flagged here because
index reconciliation blocks the whole convention.

---

## 7. Test coverage (native, in `src/lib.rs` — the emitted-sequence mirror)

The suite simulates the compiler's emitted `dup`/`drop` sequences for the representative patterns and
asserts, via the `LIVE_NODES` probe, that each returns the heap to baseline (no leak) while values
stay intact until their last owner (no early free). These are the reference behaviors the compiler's
emission must reproduce:

- `rc_convention_projection_return_dups_before_parent_drop` — §3.5/§4: extract a tuple element, keep
  it, dup-before-drop-parent; element survives, parent + siblings reclaimed.
- `rc_convention_match_extract_keeps_payload` — §3.5: `match Some(x) => x`, dup payload then drop
  scrutinee; payload survives, sum node freed.
- `rc_convention_duplicate_binder_tuple_x_x` — §3.3: `(tuple x x)` dup-once; both slots owned, one
  reclaim on tuple drop.
- `rc_convention_if_branches_balance_ownership` — §3.4: both arms leave one owned result; the
  not-taken value is dropped in each arm; no leak, no double free either way `c` goes.
- `rc_convention_dead_binding_is_dropped` — §3.1: a bound-but-unused heap value is dropped; baseline
  restored.

The reuse/FBIP tests (§8) live alongside these under the "Phase D.2: reuse / FBIP" heading.

---

## 8. Reuse / FBIP emission contract (Phase D.2 — runtime LANDED, compiler to emit)

The in-place-update optimization: when a **unique** value is consumed and a value is rebuilt in the
same breath, reuse the dying node's shell instead of free→malloc. This is Koka/Lean's core perf win
(in-place `List.map`/`filter`, record update, functional tree rebuild). The runtime ships three ops;
the compiler decides where to emit them. **Purely an optimization** — every reuse site is also
correct as a plain `drop` + fresh constructor (§1–§4). Land the baseline first; add reuse after.

### 8.1 The three ops (WIT 26–28)

- `reset(node) -> token`. The drop-to-reuse-token op. If `node` is **unique** (`rc==1`): drop the
  references it owns (a normal cascading drop of each child), **retain the emptied shell**, and
  return it as a **non-null reuse token** (same handle, now childless, `rc==1`). If **shared**
  (`rc>1`): decrement and return **0** (null token) — the other owners keep the node intact.
  Null in → null out.
- `arr-alloc-reuse(len, token) -> arr`. Like `arr-alloc(len)`, but if `token` is non-null, refit
  that shell to `len` NULL slots and return it (no allocation); else allocate fresh.
- `sum-new-reuse(disc, payload, token) -> sum`. Like `sum-new(disc, payload)`, reusing `token`'s
  shell when non-null.

The token obeys the ordinary ownership ABI: it is **consumed by exactly one** `*-reuse` constructor,
**or** `drop`ped if a control path does not rebuild (dropping a childless unique node frees exactly
the shell). No new "free the token" op — plain `drop` handles it. Passing token `0` makes the reuse
constructors behave identically to their plain forms, so a declined `reset` is fully transparent.

### 8.2 Frame-limited by construction (why this is safe)

Reuse fires **only** when `reset` sees `rc==1`. A reused shell is memory that was already live and
is about to die, so peak heap **cannot grow** from reuse (research P3/P4 — frame-limited reuse, not
algorithm D, not unrestricted borrow inference). A shared value (e.g. another version of a
persistent structure) makes `reset` return null, the rebuild allocates fresh, and the shared value
is untouched. **The compiler needs no static uniqueness proof for correctness** — `reset` checks the
count at runtime. Static uniqueness only lets you *skip the check* later (a further optimization);
the baseline emission below is correct without it.

### 8.3 The emission pattern

Replace a `drop old; new = <ctor>` pair, where `old` is dead exactly at the point `new` is built and
they have the same constructor family (array↔array, sum↔sum), with:

```
; keep any children of `old` that the rebuild carries — dup them BEFORE reset (see 8.4)
token = reset(old)                     ; old unique -> emptied shell; else null (+ decref)
new   = arr-alloc-reuse(len, token)    ; or sum-new-reuse(disc, payload, token)
; …fill `new`'s slots as usual (arr-set / already-passed payload)…
```

Highest-value sites, in order:

1. **`List.map` / `List.filter`-shaped rebuilds.** A list is a flat `arr` here (the renderer walks
   it by `arr-len`/`arr-get`). Mapping over a unique list: per the element loop, read the old element
   (borrow), compute the new one, then `reset` the old array and `arr-alloc-reuse` the shell at the
   new length and refill. The mapped list occupies the **same array shell** as the input — zero net
   node allocation for the spine. (`fbip_map_over_unique_list_reuses_in_place` proves the footprint
   is identical to the input's.)
2. **Record update `{ r | field := v }`.** `r` is an `arr`; if `r` is unique, `reset(r)` then
   `arr-alloc-reuse(arity, token)` and refill — the updated record reuses `r`'s shell.
3. **Functional recursive-sum rebuild** (cons list, tree node built with `sum-new`): `reset` the old
   node, `sum-new-reuse` the shell. This is the classic FBIP tree case.

### 8.4 The one ordering rule (identical to §4)

`reset` **drops** `old`'s references to its children. So any child of `old` that the rebuild keeps
(carries into `new`, or reads then re-stores) must be **`dup`'d before `reset(old)`** — exactly the
§4 dup-before-drop invariant, since `reset` *is* a drop that keeps the shell. If the rebuild reads a
child only *by value* (e.g. `get-int` the old element, compute a fresh leaf) and does not carry the
child handle itself, no dup is needed — `reset` reclaims the old leaf. Getting this wrong is the same
use-after-free as mis-ordering a plain drop; `reset_keeps_dup_d_child_alive_for_the_rebuild` is the
reference behavior.

### 8.5 Tests (native, `src/lib.rs`, under "Phase D.2: reuse / FBIP")

- `reset_unique_yields_emptied_shell_token` — unique reset returns the same handle as a childless
  `rc==1` token; children freed; an unused token drops to free exactly the shell.
- `reset_shared_declines_and_preserves_the_node` — shared reset returns null, decrements, leaves the
  node and its children fully intact (the frame-limiting guard).
- `reuse_ctors_with_null_token_allocate_fresh` — null token ⇒ plain allocation; declined reset is
  transparent.
- `arr_alloc_reuse_refits_the_same_shell` / `sum_new_reuse_refits_the_same_shell` — address identity:
  reuse returns the very same node, no new allocation, even across a length/shape change.
- `fbip_map_over_unique_list_reuses_in_place` — the headline: mapping over a unique list yields a
  result with the **same node footprint** as the input; peak = input + transient new leaves, not
  doubled.
- `reset_keeps_dup_d_child_alive_for_the_rebuild` — §8.4: a dup'd kept child survives `reset` into
  the reused shell.
