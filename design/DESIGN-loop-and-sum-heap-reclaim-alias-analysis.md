# Heap reclaim for self-loops and heap-payload sums — the alias analysis both need

Status: DESIGN (v-runtime, 2026-08-27). Seeds a v-core-opt / Perceus-level effort. Goal: reclaim the
two BIGGEST remaining leak classes — recursion/loop/fold (~333 cases) and heap-payload sum-match
(~195 cases) — which today leak because a **shell/old-value drop is suppressed** to stay clear of a
use-after-free. Both need the SAME thing: an alias/ownership check strong enough to prove the drop is
safe. Incremental gate tweaks by v-runtime have twice hit real UAFs (traps), so this is written as a
design seed with worked witnesses rather than a landed patch.

Motivation: gate `--check` now default-enforces leak-freedom (#3808); the ~1605 grandfathered
known-leaks retire as they are reclaimed. These two classes are ~528 of that 1605 — the single
largest lever — and the operator wants the count driven toward zero.

## The two leak sites

**A. Self-loop-tail (recursion/loop/fold, v-nix ROI ~333).** A tail-recursive walk compiled to a wasm
loop leaks its walked heap param one cell per iteration:

```
(def (go (: xs (List Int64)) (: acc Int64))
  (match xs ((list) acc) ((list h .. t) (go t (+ acc h)))))   ; leaks ~2/iteration
```

Root cause (v-runtime tick-as, emitted-wasm-verified): the self-loop back-edge is a `br` to the loop
top, which **bypasses the post-match shell reclaim**. `list_shell_reclaim_slot` explicitly returns
`None` for `TailPos::Tail(Some(_))` (the self-loop case) — the arm never reaches the post-match drop,
and the scrutinee-stash slot is reused next iteration. So the old list node is never reclaimed. A
naive "drop the old slot value before the back-edge reassign" does NOT work: the walked param's
refcount at the back-edge is already ≥ 2 (the loop body dups it for its multiple uses — match
scrutinee, head read, rest read), so a single `drop` never reaches 0. RULED OUT across 3 attempts.

**B. Heap-payload sum-match (~195 cases).** A `MatchSum`/`SumExpect` whose scrutinee is an owned
temporary with a HEAP payload leaks the payload chain when the arm destructures it:

```
(match (if c (Option.Some (list a b)) Option.None)
  ((Option.Some (list a b)) (+ a b)) (_ -1))                  ; leaks 3 (payload list chain)
```

Root cause: the shell-reclaim gate (`sum_has_only_scalar_payloads`) refuses any heap payload. The
2026-07-19 "inc2" broadening ("any owned boxed sum + dup the consumed children") was reverted as
UNSOUND. v-runtime re-attempted it (tick-27a) gated by `arm_borrows_heap_subvalue` (reject an arm
that materializes a heap **borrowing projection** of the payload): it reclaimed ~126 cases to zero
(+66 partial) — but **still trapped 3 cases** (`mts1`, `mmx1`, `rrb1`), so it was reverted. Position-
independent and representation-shared: the same leak appears for `Result`, `Option`-in-`Option`, and
inside handler arms (breaker's `rs1`/`rs2`/`ap1`), so ONE match-lowering fix site should cover all.

## The shared safety invariant

A shell (sum shell / walked list node) may be deep-`drop`ped at a program point iff **no reference
into it is live past that point.** The existing scalar-only floors (`sum_has_only_scalar_payloads`,
list `!is_heap_type(elem)`) are the trivially-safe subset (a scalar payload copies out, holding no
handle). The reclaimable-but-currently-declined cases are exactly those where a heap payload is
DESTRUCTURED to scalars — no live handle survives — yet the analysis cannot prove it.

## Why the current alias check is insufficient (the negative-space data)

`arm_borrows_heap_subvalue` (the lm3/msr6 detector) flags an arm that materializes a heap
**borrowing projection** (`arr-get`/`vec-get`/`sum-payload` returning a heap handle) in a non-borrow
position. It correctly rejects the classic sread-UAF — `((Arena m _) (Map.lookup m id))` — where a
payload child is aliased OUT via `Map.lookup`. But three trap witnesses prove it misses other alias
paths:

- `mts1` — a `Map` whose VALUES are tuples; the arm does a **tuple REBUILD** and "packs the fresh
  pair". The rebuild reuses/aliases the payload cell (FBIP in-place reuse), which is NOT a projection
  the detector sees.
- `mmx1` — `Option (Tuple min max)` threaded as handler STATE; the compound is carried across
  `resume` and read after the match.
- `rrb1` — a round-robin scheduler threading compound state.

**The key insight (breaker): the alias detector must see REBUILDS / FBIP reuse, not just
projections.** A drop is unsafe not only when a payload handle is read out, but when the payload's
CELLS are reused into the result (FBIP) or threaded into state that outlives the match.

## What the analysis must decide

For a candidate shell drop (sum shell after the match arm; walked list node before the loop
back-edge), prove that at the drop point **every cell reachable from the shell is dead** — i.e. no
result value, no other live binding/slot, and no effect boundary holds a reference into it, accounting
for:

1. **Projections** — a heap `sum-payload`/`arr-get`/`vec-get` read out as a live handle (covered by
   `arm_borrows_heap_subvalue`).
2. **FBIP / rebuild reuse** — the Perceus reuse token: if the arm rebuilds a compound by reusing the
   payload's cell, that cell is aliased into the result. (The gap that trapped `mts1`.)
3. **Threaded / escaped state** — the payload flowing into `resume`, a return, a constructor, or a
   non-tail call (covered for consumes by `binding_escapes`, but `binding_escapes` models a match as
   CONSUMING its scrutinee, which is too coarse for the net-zero loop read — see A).
4. **Loop back-edge liveness (site A)** — the walked param is dup'd for multi-use; the reclaim must
   drop the balance so each iteration nets zero, on the `br` path (which the post-match drop misses).

This is a liveness-at-drop-point analysis over the Perceus dup/drop/reuse placement — v-core-opt's
lane. The scalar-only floors are the current sound under-approximation; the goal is to widen them to
the destructure-to-scalar cases without admitting an FBIP/threaded alias.

## Acceptance set + fence (ready witnesses, all in the corpus queue)

- **Reclaim-to-zero (must become `live-objects 0`):** `d4` (minimal Option), `dm1`/`d3` (scaling),
  `rs1` (Result twin), `rs2` (nested Option), `ap1` (arm/handler position); plus the self-loop
  `fold`/`count` family for site A.
- **Fence (must STAY correct — guards over-correction):** `dst2`/`dst5`/`dst6` (user-sum controls),
  `rs3` (whole-binding), and the three trap witnesses `mts1`/`mmx1`/`rrb1` (must NOT be reclaimed
  until the analysis proves them safe — a leak is strictly better than the UAF they take today).
- Repros: `queue/adv-option-nested-payload-destructure-leak.sexp` (breaker) + the fold baselines.

## Non-goals / discipline

- A leak beats a UAF. Until the analysis is complete, these families stay `(live-objects known-leak N)`
  (breaker banked them); the markers retire only when a witness provably reclaims.
- No incremental gate tweak that widens the reclaim without the rebuild/FBIP-aware alias check — twice
  it has trapped. The fix is one place (the shared shell-reclaim/loop-back-edge decision) fed by the
  complete analysis.
