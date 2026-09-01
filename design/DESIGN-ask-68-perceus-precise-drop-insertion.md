# 68. Perceus precise drop insertion — the full, spelled-out plan (M2 Phase D / "task #9")

**Status: 🟢 READY TO IMPLEMENT. Fully scoped below — every phase is independently gate-green,
leak-oracle-measurable, and safe against double-free. Do the phases in order; each is shippable.**

**One-paragraph orientation.** Today the compiler is crash-safe but leaks: `gen_name` emits a `dup`
after *every* `Kind::Heap` local read (codegen.rs ~3203) and emits essentially **no drops** (the only
real `himport::DROP` is the hand-written one in `gen_runtime_bytes_slice`'s out-of-bounds arm,
codegen.rs ~7937). So a heap value's owning reference is never reclaimed. The leak oracle
(`live-objects`, WIT #54) makes this visible: `(List.len (List.push (List.push (list) 5) 7))` →
value `2`, **`live-objects → 4`**; the ask-63 twice-consumed `(both (list))` → value `12`,
**`live-objects → 13`**. This ask drives those to **0** without regressing any value.

**The reframe that makes this tractable (READ THIS FIRST).** The memory/lore calls precise drops "the
hard part" because it conflates two separable goals:
- **Leak-freedom (correctness):** `live-objects → 0` after every run, no double-free, no UAF. This is
  reachable with **NO liveness/last-use analysis** — see Phases 1–2. It's mostly mechanical.
- **Optimality (performance):** minimal rc traffic ("map over a unique list allocates nothing"). This
  is the part that needs last-use analysis (Phase 3) and FBIP reuse (Phase 4). Both are **optional**
  and can be deferred indefinitely without any leak.

So the correctness target — the thing the operator actually wants (`live-objects → 0`) — lands at the
**end of Phase 2**, and Phase 2 requires only a mechanical owned-vs-borrowed context flag threaded
through `emit`, not a dataflow analysis. Do not let the "Perceus is hard" lore scope-creep this.

---

## Background facts (verified against the current tree — cite these, don't re-derive)

### The runtime rc contract (all ops in `crates/cdz-runtime/src/lib.rs`)

`alloc()` (lib.rs ~142) births every node at **rc = 1**. `op_dup` (~539) is a **shallow header-only**
`+1` (never recurses). `op_drop` (~566) decrements; at rc→0 it frees the node and **iteratively**
cascades into owned children (shared children `rc>1` just decrement and survive). **`drop(NULL)` is a
benign no-op** (lib.rs 569) — this is load-bearing for Phase 1 (dropping an unassigned local is safe).

**Producers** (return a FRESH owned handle, rc=1): `box-int/bool/float`, `arr-alloc`, `sum-new`,
`bytes-alloc`, `vec-empty`, `vec-push`, `vec-update`, `bytes-concat`, `bytes-slice`, `map-empty`,
`map-insert`, `map-remove`.

**Consumers** (take ownership of a heap arg — caller must NOT drop it after): `vec-push`/`vec-update`
(consume the list **and** the boxed elem), `sum-new` (consumes payload), `bytes-concat` (both),
`bytes-slice`/`bytes-compact` (the buffer), `arr-set`/`map-set` (store the elem/key/val **without
dup**), `map-insert`/`map-remove` (consume the map).

**⚠ BORROWED-return accessors — the UAF class. These return the stored child handle WITHOUT bumping
its rc** (the returned handle points *into* a structure the caller doesn't own a fresh ref to):
`arr-get` (232), `sum-payload` (254), `map-key` (510), `map-val` (519), `vec-get` (~1034),
`map-lookup` (~2086), `map-iter-key` (~3157), `map-iter-val` (~3166). The runtime's OWN test suite
prescribes the required compiler discipline: `rc_convention_projection_return_dups_before_parent_drop`
(~3992) and `rc_convention_match_extract_keeps_payload` (~4015) both do **`dup(kept); drop(parent)`**
in that order. **This is the single most important fact in this ask.** A projected element that
outlives its parent MUST be `dup`'d before the parent is dropped, or it's a use-after-free.

**Reads** (borrow, return a scalar — no handle, no ownership): `get-*`, `arr-len`, `sum-disc`,
`bytes-len`, `bytes-get`, `vec-len`, `map-len`, `map-size`, `set-contains`, `set-size`.

### The runtime is AHEAD of the compiler: FBIP reuse is already built (Phase 4 is pre-staged)

`reset` (WIT 26, lib.rs ~626), `arr-alloc-reuse` (WIT 27, ~659), `sum-new-reuse` (WIT 28, ~674) are
**implemented and native-tested** in the runtime but are **NOT in the compiler's `HEAP_ALLOWLIST`**
(`xtask/src/wit_envelope.rs:38`). So Phase 4 (in-place reuse) is a wiring + emission job, not a
runtime job. `vec-push`/`vec-update`/`map-insert` already do their OWN internal FBIP (reuse the shell
when the header is unique), so list/map growth is already allocation-optimal on the unique path — the
reuse ops matter for user-level tuple/record/sum rebuilds.

### Where heap values enter/leave scope in `codegen.rs` (the insertion map)

Function names are the stable anchor — **search by name**, line numbers drift.

| Construct | Function (anchor) | Ownership fact |
|---|---|---|
| Fn params `Kind::Heap` | `compile_func` (env built ~1945) | Callee OWNS them (non-alias `Local::scalar_shaped`, locals `0..arity`). No drop at return today → leak. |
| Materialized let-local | `gen_let` (`alloc_local`+`LOCAL_SET` path, ~5126) | OWNS one handle; lives to function end; never dropped → leak. Shape recorded when `ek==Heap` (~5134). |
| let alias fast-paths | `gen_let` (~5073/5085/5110/5120) | Own NOTHING — re-emit the node per use. Never drop for the binding; but each *use* re-runs a constructor producing fresh owned garbage. |
| do-block non-final | `gen_do` (~5106) | **BUG: emits `op::DROP` (wasm stack-drop) for a discarded `Kind::Heap` value → discards the handle bits WITHOUT decrementing rc → leak.** Must become `himport::DROP`. |
| match scrutinee | `gen_match_runtime` / `gen_match_runtime_sum` | Scrutinee `handle` materialized (`alloc_local(Heap)`), OWNED, read for `sum-disc`/`sum-payload` (borrows). Never dropped → leak. |
| payload binder (name) | `bind_sum_payload_kinds` name-arm | `sum-payload(handle)` (BORROW) → stored in a binder local. Binder must be dup'd (owned) & dropped. |
| payload binder (whole, catch-all) | `gen_sum_arms` bare-name arm | ⚠ ALIASES the scrutinee's local idx (two names → ONE handle). Dropping both double-frees — treat as ONE ownership slot. |
| payload binder (tuple destructure) | `bind_sum_payload_kinds` tuple-arm → `bind_tuple_elems` | `sum-payload` then per-slot `arr-get` (BORROW each). Nested containers → tree of simultaneously-live borrowed handles. |
| fn call args | `gen_call` (~8439) | Heap arg passed → callee OWNS it. Today the arg came via a dup'd read, so caller keeps its ref too (the over-retain). |
| inlined call (effect/HOF/poly) | `gen_call` (~8483/8502), `gen_apply` | Params are ALIASES to arg nodes — no handoff; each param-use re-emits the arg. Ownership multiplicity = number of textual uses, not 1. |
| if branches | `gen_if` (~4922) | Only one arm runs; both emitted. Per-branch drop balance required (a local live in one arm only needs a drop in the other). |
| runtime constructors | `gen_runtime_ctor`/`_sum`/`_list_*`/`_string_*`/`_bytes_concat` | Birth points — produce a fresh owned handle; consuming-op operands are MOVED in. |
| projections (READS) | `gen_tuple_access`, `gen_member`/`gen_runtime_member`, `gen_runtime_list_at`, `gen_runtime_bytes_at` | `arr-get`/`vec-get` return a BORROWED element; the parent stays owned by its holder. |

**Maps/sets are compile-time-only today** (`Map.*`/`Set.*` fold in `eval_const`; no runtime map/set
ops are emitted). So no drop insertion is needed for them until they get a runtime representation —
but write the borrow/consume rules generically so they apply when that lands (ask-60).

---

## The ownership model (the discipline every phase upholds)

**Invariant:** at any program point, every live heap value has **exactly one owning reference**; every
owning reference is eventually consumed exactly once — by a consuming op/call, by becoming the
function result, or by an explicit `drop`.

**Two occurrence contexts.** Every position where a heap value is emitted is either:
- **OWNED** — the consumer takes ownership: a call argument, a consuming-op operand
  (`vec-push`/`sum-new`/`bytes-concat`/…), a let-RHS stored into a local, an `if`/match-arm/do-final
  **tail** value, the function result.
- **BORROWED** — the consumer only reads and does not take ownership: an accessor-op argument
  (`arr-get`/`sum-payload`/`vec-get`/`map-lookup`/…), a scalar-read argument
  (`arr-len`/`sum-disc`/`bytes-len`/`vec-len`), and equality operands (`gen_eq`'s runtime compare).

**The rules (this IS Perceus, specialized to this emitter):**
1. **Heap local in OWNED context** → `dup` it (Phase 1–2 keep the current "always dup"; Phase 3 makes
   it "dup unless last-use").
2. **Heap local in BORROWED context** → **NO dup** (the local keeps ownership; the borrow is
   transient). *This is the fix for the stranded-accessor leak.*
3. **Accessor op with a heap RESULT** → emit its parent argument in **BORROWED** context; then, if the
   caller's context is OWNED, emit `dup` on the result (turn the borrow into an owned ref); if BORROWED
   (nested projection), no dup.
4. **Scope exit** (function end) → `drop` every materialized heap local that still owns its reference
   (Phase 1). Under rules 1–3 the owning ref is never consumed by a read, so this drop is always
   balanced and safe — including for a local whose value was returned (the result is a separate dup'd
   ref).

**Why leak-freedom needs no liveness analysis:** with rule 1 = "always dup on owned read" and rule 4 =
"drop every heap local at function end," every owned read is `+1 dup`, matched by either the consumer
(`-1`) or the scope-end drop of the owning ref (`-1`). Borrowed reads (rule 2) touch nothing.
Accessor results (rule 3) are dup'd into owned refs that then follow the same discipline. Net rc change
across a function is exactly zero for every value that doesn't escape as the result → **`live-objects
→ 0`** with only mechanical, non-dataflow transformations.

---

## Phase 0 — Instrumentation & the test-first spine (do this first; ~half a day)

The leak oracle is the whole point — wire it into the gate so every subsequent phase is measured, not
asserted.

**0.1 Build the counter-enabled runtime and pin it.**
```
export PATH="$HOME/.cargo/bin:$PATH"
cd implementation/seed/crates/cdz-runtime
cargo component build --release --target wasm32-unknown-unknown --features debug-counters
# ⚠ MUST be `cargo component build`, NOT `cargo build` — plain build emits a bare core module and
#   every heap case fails with "value-heap runtime component invalid" (looks like a mass regression).
export CADENZA_RUNTIME=$PWD/target/wasm32-unknown-unknown/release/cdz_runtime.wasm
```
Verify: `cd implementation/seed && cargo run -p cadenza-seed -- emit <a-heap-program.cdz>` prints a
`live-objects → N` line (host reads it in `run_with_runtime`, host.rs ~381; printed in main.rs ~178).
The default runtime returns a constant 0, so this line only carries signal against the debug build.

**0.2 Add a `(live-objects N)` corpus clause** (harness work — the infra is 90% there):
- `crates/cadenza-seed/src/corpus.rs`: add `live_after: Option<u32>` to `Case` (struct ~36); parse a
  `(live-objects N)` clause in `parse_case` (~130–231, add an arm near the `needs` one ~185); in
  `compare` (after the primary-result check, ~504) assert `state.live_after_run == Some(expected)`
  **when the clause is present AND `state.live_after_run.is_some()`** — skip the assertion when it's
  `None` (default runtime) so the normal gate is unaffected. `state.live_after_run` already flows into
  `compare` via the `state` arg (corpus.rs ~409/422).
- Gate the assertion behind the debug runtime being pinned: when `CADENZA_RUNTIME` points at the
  counter build, `live_after_run` is `Some`, so the clause fires; otherwise it's inert. (The
  documented `CADENZA_LEAK_CHECK` env var is **not implemented** — don't rely on it; this `Some`-gating
  is the mechanism.)

**0.3 Author the RED baseline cases** (these document today's leaks; each becomes GREEN as phases land):
Add to the relevant `spec/semantics/*.sexp` files, each with its known baseline count as a
**deliberately-red** target you'll edit to `0`:
- push+len: `(List.len (List.push (List.push (list) 5) 7))` → value 2, `(live-objects 4)` today.
- ask-63 twice-consumed: the existing `(both (list))` case in 05-compound-types.sexp ~2362 → value 12,
  `(live-objects 13)` today. (Add the clause to the existing case.)
- tuple projection: `(tuple.0 (tuple (List.push (list) 1) 9))` — exercises borrowed `arr-get`.
- sum match: a `(match (Some (list)) ((None _) 0) ((Some xs) (List.len xs)))` — scrutinee + payload.
- nested list, bytes-concat, record field — one each.

Run `cargo run -p cadenza-seed -- behavior-gate ../../spec/semantics` with the debug runtime pinned:
these cases FAIL on the `live-objects` clause (nonzero). That's the target list. **Definition of done
for the whole ask = every one of these reads `(live-objects 0)` and the value stays correct.**

---

## Phase 1 — Drop materialized heap locals at function end ("balance the dups")

**Change:** keep dup-on-read exactly as-is; add scope-end drops.
- Add `heap_locals: Vec<u32>` to `FnCtx` (struct ~10093).
- Push the local idx whenever a **materialized** `Kind::Heap` local is created:
  - `compile_func`: for each param with `param_kinds[i] == Kind::Heap` (env build ~1945).
  - `gen_let`: in the `alloc_local`+`LOCAL_SET` path (~5126), when `ek == Kind::Heap`.
  - `gen_match_runtime_sum`: the scrutinee `handle` local, and each materialized heap payload/binder
    local in `bind_sum_payload_kinds`/`bind_tuple_elems`.
  - **Do NOT push alias locals** (structural/scalar-literal/folded-compound/unit — they own nothing).
  - **Dedup by idx** (a `BTreeSet<u32>` is cleaner): the catch-all whole-payload binder
    (`gen_sum_arms` bare-name arm) aliases the scrutinee's idx — pushing it twice would double-free.
- In `compile_func`, after `emit` returns `(code, kind)` and before building the `Body`: append, for
  each idx in `heap_locals`, the bytes `local.get idx; call himport::DROP`. The function result is
  already on the stack; these drops pop only their own arg, so the result stays on top. Emit them
  **after** the body code, before the implicit `end`.

**Why it's safe (stated for the reviewer):** every read of a heap local dups (owning ref never
consumed), so the owning ref is always intact and droppable exactly once. A local unassigned on the
taken path is `0`/NULL, and `drop(NULL)` is a no-op (lib.rs 569) — so dropping every heap-typed local
unconditionally at function end is safe even for locals assigned on only one branch. A returned local's
result is a separate dup'd ref, untouched by dropping the owning ref.

**Expected oracle movement:** local-shaped leaks close; push+len and ask-63 drop substantially (not to
0 yet — borrowed-accessor strands remain). Value gate stays GREEN (dropping an unconsumed owning ref
never changes a value). Any double-free surfaces as a TRAP in the oracle (over-drop → `op_drop` →
`talc`) — if that happens, you pushed an alias idx or a non-owning idx into `heap_locals`.

**Also fix the do-block leak here** (it's the same "reclaim a discarded owning ref" shape): in
`gen_do` (~5106), when the discarded non-final form's `fk == Kind::Heap`, emit `himport::DROP` instead
of `op::DROP`. (`op::DROP` on a heap value discards the handle bits without decrementing rc.) Keep
`op::DROP` for scalar `fk`.

---

## Phase 2 — Owned/Borrowed contexts (this is where `live-objects → 0` lands)

**Change:** introduce the context distinction from the model above and stop dup'ing in borrow
positions; dup escaping accessor results.

- Add `enum OwnCtx { Owned, Borrowed }` and thread it as a parameter through `emit` (or add a sibling
  `emit_borrowed` that sets a flag — a threaded param is cleaner and future-proofs Phase 3). Default
  everywhere is `Owned`; the current call sites all become `Owned` (byte-identical to today for
  non-heap and for owned heap reads, since owned still dups in Phase 2).
- **`gen_name`** (~3172): the `Kind::Heap` local branch (~3203) dups **only when ctx == Owned**. When
  `Borrowed`, emit the bare `local.get idx` with no dup.
- **Accessor emitters emit their parent in `Borrowed`:** `gen_tuple_access` (arr-get), `gen_member`/
  `gen_runtime_member` (arr-get by field slot), `gen_runtime_list_at` (vec-get), `gen_runtime_bytes_at`,
  the match scrutinee reads in `gen_match_runtime_sum` (sum-disc/sum-payload), and the scalar-read
  argument of `List.len`/`arr-len`/`bytes-len`. Equality operands in `gen_eq`'s runtime path →
  `Borrowed`.
- **Accessor heap RESULT** (rule 3): in each accessor emitter, after the `arr-get`/`sum-payload`/
  `vec-get`/`map-lookup` call, if the element is `Kind::Heap` **and** the caller's ctx is `Owned`, emit
  `call himport::DUP` on the result (turning the borrow into an owned ref). If the caller's ctx is
  `Borrowed` (nested projection like `(tuple.0 (tuple.1 t))`), skip the dup — the outer accessor
  borrows it. This is exactly the runtime's prescribed `dup(kept); drop(parent)` order: the parent's
  drop is Phase 1's scope-end drop; the kept element's dup is here.
- **Payload binders** (`bind_sum_payload_kinds`): a binder stored into its own local is an OWNED
  context for the `sum-payload`/`arr-get` result → dup after the accessor (so the binder-local owns a
  real ref), then Phase 1 drops the binder-local at scope end. The scrutinee `handle` stays owned by
  its own local (borrowed for the disc/payload reads), dropped by Phase 1. The whole-payload catch-all
  binder that aliases the scrutinee idx stays ONE slot (already deduped in Phase 1).

**Why `live-objects → 0` now:** owned reads dup and are balanced by consumer-or-scope-drop; borrowed
reads touch nothing; accessor results become owned refs that are stored/consumed/dropped like any other
heap value; parents are dropped once at scope end. Every non-escaping value nets to zero rc.

**⚠ Two correctness watch-items for this phase:**
1. **The final result.** If `main` returns a **compound** heap value, the type-directed renderer walks
   it via borrowed accessors and then nobody drops the top-level result → `live-objects` shows the
   result's node count, not 0. Fix: after the renderer finishes (host render path / the `run` result
   handling), `drop` the top-level result handle once. Scalar-returning programs (push+len → Int) have
   no residual, so this only matters for compound-returning mains — but the oracle cases must account
   for it. Check host.rs `run_with_runtime` / the render entry.
2. **Branch imbalance** (`gen_if`, and `gen_ctor_arm`'s shared-`else_c`): a heap value produced/owned
   in one arm but not the sibling must be balanced. Under the "drop all heap locals at function end"
   discipline this is mostly automatic (a local assigned in one arm is NULL on the other → benign
   drop). But watch `gen_ctor_arm` where the sibling `else_c` is computed once and **textually
   embedded in every arm's else** — a drop naively placed in `else_c` gets duplicated across arms
   though only one runs. Keep drops at the function-end sweep (Phase 1), NOT inside `else_c`, and this
   is avoided.

**Gate at end of Phase 2:** every Phase-0 baseline case reads `(live-objects 0)` with correct values;
full behavior-gate GREEN; `component-check` byte-gate still GREEN (these are runtime-behavior changes to
emitted heap programs — the compiler's OWN bytes only change where it emits heap ops, so re-verify the
self-host byte-gate). **Correctness is DONE here.** Ship it. Phases 3–4 are optimization.

---

## Phase 3 — Precise last-use (OPTIONAL optimization; the genuinely hard part)

Not needed for leak-freedom — Phase 2 is already `live-objects → 0`. This phase removes the redundant
`dup`-then-scope-`drop` churn so an owned value used once is *moved*, not copied-and-reclaimed
(realizing "map over a unique list allocates nothing" and tightening byte-identity against cdz-rustc).

- Compute per-scope last-use: for a heap local occurrence, is the local read again in the remainder of
  its scope? A syntactic "does this name appear in the remaining forms of the enclosing let/do/arm/
  body" check suffices (scopes are small; the compiler already re-walks — but re-use the O(1)
  `expect_name_only` machinery from the compile-cost fix, don't reintroduce the 2ⁿ let-nesting blowup;
  see [[compile-cost-exponential-let-if-nesting-fixed]]).
- Owned read at **last use** → move (no dup); then that local must **not** be dropped at scope end
  (remove it from the `heap_locals` sweep on the paths where it was moved). This is where **double-free
  risk enters** — an under-count/mis-placed move that drops-and-consumes traps immediately in the
  oracle. Drive it case-by-case against `live-objects` + the trap check.
- Frame-limited only (research **P3**): do NOT add unrestricted borrow inference — it's not
  frame-limited and blows up peak heap. Keep borrowing to the syntactic read-only positions of Phase 2.

**Companion — UAF safety net (ask-64):** before trusting Phase 3's moves broadly, land generation-
tagged handles (ask-64, runtime-owned, `debug-uaf` feature) so a mis-placed drop that frees a
still-referenced value TRAPS deterministically instead of silently corrupting. The value oracle catches
wrong values and the live-objects oracle catches leaks + double-frees, but a UAF that reads stale-but-
mapped memory is caught by neither until ask-64.

---

## Phase 4 — FBIP reset/reuse (OPTIONAL optimization; runtime already built)

Wire the pre-built reuse ops so a unique value consumed-and-rebuilt reuses its shell instead of
free→malloc (research **P2**). Runtime side is DONE (`reset`/`arr-alloc-reuse`/`sum-new-reuse`, WIT
26–28, native-tested).

- Add `"reset"`, `"arr-alloc-reuse"`, `"sum-new-reuse"` to `HEAP_ALLOWLIST` (`wit_envelope.rs:38`),
  run `cargo run -p xtask -- build` to regenerate `runtime_funcs.rs`/`heap_envelope.rs`/the envelope,
  re-verify gates. ⚠ Appending shifts nothing if added at the end, but these have fixed WIT indices
  26–28 that sit *before* the CHAMP block — confirm the generated `himport` indices and envelope match
  the WIT and that `component-check` stays byte-GREEN (the allowlist ORDER is the index assignment).
- Emit the two-step protocol at reuse sites (tuple/record/sum rebuild where the source is provably
  unique): `token = reset(old)` (drops old's children refs, returns the emptied shell if unique else
  null), then `arr-alloc-reuse(len, token)` / `sum-new-reuse(disc, payload, token)`. **Any value that
  survives into the new value must be `dup`'d BEFORE `reset(old)`** (lib.rs ~616) — reset drops old's
  references to its children. `vec-push`/`vec-update`/`map-insert` already self-reuse, so this is for
  user-level fixed-arity rebuilds only.
- Reuse fires ONLY on `rc==1` (frame-limited, research P3/P4) → peak heap cannot grow. Deterministic
  function of the source (spec: memory-and-resource-model.md §Reuse Is Not Observable / §A Decision To
  Reuse … MUST Be A Deterministic Function Of The Source).

---

## Definition of done & spec trace

- **Correctness (Phases 1–2, the actual ask):** every Phase-0 leak-oracle case reads `(live-objects
  0)` with correct values; behavior-gate GREEN; `component-check` byte-gate GREEN. Realizes
  memory-and-resource-model.md §Cleanup Is Source-Determined ("released after its last use") and
  §Reclamation Is Carried By The Runnable Form (no collector; rc reclaims the acyclic heap).
- **Optimization (Phases 3–4):** deferred, independently shippable, guarded by ask-64's UAF trap for
  Phase 3.

## Watch-list (the traps that will bite)
1. Alias locals must never enter `heap_locals` (own nothing) — else scope-end drop double-frees.
2. The whole-payload catch-all binder aliases the scrutinee idx — ONE slot, dedup.
3. `gen_ctor_arm`'s shared `else_c` is textually duplicated across arms — keep drops in the function-end
   sweep, never inside `else_c`.
4. Compound-returning `main` needs a top-level result drop after render, or `live-objects` ≠ 0.
5. Nested projections `(tuple.0 (tuple.1 t))` — the inner result is BORROWED (outer accessor's arg), so
   don't dup it; only dup an accessor result whose caller context is Owned.
6. Build the runtime with `cargo component build --features debug-counters`, pin `CADENZA_RUNTIME`, or
   the oracle reads a constant 0 and every case falsely passes.
7. Phase 3 last-use must reuse the O(1) name-scan machinery — don't reintroduce the 2ⁿ let-nesting
   compile-cost blowup.

Related: [[heap-local-dup-before-consume-unblock]] (ask-63, the crash-safe unblock this completes),
[[live-object-count-leak-oracle]] (the oracle), [[rc-heap-persistent-ds-sota-2026-07-05]] (the research
P1–P10), ask-64 (UAF trap), ask-60 (runtime maps/sets → their drop rules), memory-and-resource-model.md.
