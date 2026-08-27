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

## The Perceus-literature technique (operator-directed 2026-08-27)

The operator pointed us at the Perceus / Koka literature before we invent a bespoke invariant — this is a
solved problem there, and the papers reframe it in a way that dissolves the "static alias proof" we kept
failing to build. Sources: **Perceus: Garbage-Free Reference Counting with Reuse** (Reinking, Xie, de Moura,
Leijen — PLDI 2021); **Reference Counting with Frame-Limited Reuse** (Lorenzen & Leijen — ICFP 2022, the
`borrow` extension); **FP²: Fully in-Place Functional Programming** (Lorenzen, Leijen, Swierstra — ICFP 2023);
FBIP as described in the Koka docs.

**The reframe.** Perceus never *statically suppresses* a drop to stay clear of a UAF. It **always drops an
owned value at its last use**, and makes the free *runtime-conditional* on `rc == 1` — exactly what our
`op_drop` already does (cascade-free at rc 1, decrement otherwise). Soundness comes from the **dup/drop
balance**, not from a static liveness proof:

- A matched owned value is *consumed* by the match. Every binding extracted from it that **outlives the
  match** — escapes into the result, into threaded/handler state, into `resume`, into a rebuilt constructor,
  or into the next loop iteration — is **`dup`'d at its bind site** (ownership transfer). Given correct dups,
  dropping the shell is *always* sound: the runtime frees only genuinely-unique cells and leaves any
  still-referenced payload alone.
- **Reuse (FBIP)** is then an *optimization*, not a safety gate: `drop-reuse x; Con(…)` fuses into an
  in-place update **iff `x` is unique at runtime** (the reuse token is null otherwise and `Con` allocs
  fresh). It is never load-bearing for soundness.

**Why our two attempts trapped.** We treated `arm_borrows_heap_subvalue` (and the missing FBIP-rebuild
detector) as a switch to *decline* the shell drop. Perceus says the opposite: if a payload is aliased out /
rebuilt / threaded, it will have been **`dup`'d**, so dropping the shell is safe regardless. `mts1` (tuple
rebuild), `mmx1`/`rrb1` (threaded compound state) trapped because we widened the *drop* without inserting the
corresponding *dup* on the escaping payload — a **missing dup**, not a should-not-drop. The bug is dup
placement, not a liveness gap.

**The adopted invariant (feeds v-core-opt's dup/drop-placement lane):**

> Drop a matched owned shell at its last use. `dup` every binding projected out of it that escapes the match
> (result / state / resume / rebuild / next iteration). Emit the shell drop *unconditionally* — the
> runtime-conditional free (rc==1) is the safety net; FBIP reuse is the fast path when unique.

This is precisely breaker's ref-accounting ("every bind-dup pairs with an arm-exit release; unbalanced =
(#unwraps−1) + (1 if extraction)") stated as the Perceus dup-at-bind / drop-at-last-use rule. It explains
both sites uniformly:

- **Site A (loop back-edge).** `match xs { Cons h t -> go(t, …) }`: the match consumes `xs`; `t` (and any
  used `h`) is dup'd because it escapes into the recursive call; the **cons shell** is then dropped on the
  `br` path — a *shell* decrement (children already owned via their dups), not a deep drop of the whole
  list. That nets exactly one cons cell freed per iteration. Our naive slot-drop failed because it deep-drops
  a whole-list slot whose rc≥2 (the multi-use dups), never reaching the shell. `list_shell_reclaim_slot`
  must emit the shell drop for `Tail(Some(_))` too, and the escaping `t` must be dup'd — which it already is.
- **Site B (heap-payload sum-match).** Same rule: dup the payload where it escapes/rebuilds/threads, then
  drop the sum shell unconditionally. `mts1`/`mmx1`/`rrb1` become sound *because* the rebuild/threaded
  payload is dup'd, so the shell drop can't free a live cell. Repeat-unwrap (`xop4`: matched twice) leaks
  today because each match dups the payload but only **one** shell-drop fires — the fix is a per-match shell
  drop, in the *general* `MatchSum` path (not per-sum-kind — breaker #3892 proved `Result`/user-generic leak
  the identical 3).

## Acceptance set + fence (ready witnesses, all in the corpus queue)

- **Reclaim-to-zero (must become `live-objects 0`):** `d4` (minimal Option), `dm1`/`d3` (scaling),
  `rs1` (Result twin), `rs2` (nested Option), `ap1` (arm/handler position); plus the self-loop
  `fold`/`count` family for site A. Finalized B-prime surface (breaker #3892): extraction
  `{lar1,mlr1,mlr2,xar1,xar3,osx3,xop2,xop3}` + repeat-unwrap `{xop1,xop4,xop5,ruw1,ruw2,ruw3}` → 0.
- **Fence (must STAY correct — guards over-correction):** `dst2`/`dst5`/`dst6` (user-sum controls),
  `rs3` (whole-binding), `lar2`/`mlr3` (borrow-only shell controls), `d6` (deforestation control),
  `dt2`, and the three trap witnesses `mts1`/`mmx1`/`rrb1` (with correct dup placement these become
  *reclaimable*, not fenced — but they must never be freed while a payload alias is live; a leak beats
  the UAF until dups are proven correct).
- **Census caveat (breaker #3892):** `live-objects` counts OBJECTS, not refs. A half-balanced
  implementation can leave a stray unbalanced ref that is *invisible* to the reading until it pins
  additional structure (`ruw3` — fresh Option matched three times — still reads 3, not 4+, because the
  extra ref doesn't pin a distinct object; `xop1` reads 6 because its extraction ref pins the container
  graph). Acceptance therefore needs one **ref-level** assertion (or a witness shaped so every stray ref
  pins distinct structure) to catch a half-balanced dup/drop — an unchanged object count is not
  sufficient proof of balance.
- Repros: `queue/adv-option-nested-payload-destructure-leak.sexp` (breaker) + the fold baselines.

## Site B, one gate, two bugs (v-core-opt inc2a + B-select, LANDED 2026-08-27)

Site B is a SINGLE emit site (`emit_sum_cont`, the tail `MatchSum` shell-reclaim gate) that was declining for
TWO independent reasons. Both are now fixed and landed; the whole §B acceptance (and far more) reclaims.

- **inc2a (`emit_sum_cont` gate widen; #3943 + #3961, landed):** widened the shell-reclaim gate past the
  all-scalar floor to admit a compound heap payload when escape-clean + reuse-clean + not-extraction-retained.
  Reclaims **61 corpus cases across 15 files** — the COMPLEX/recursive-sum family (deep-nested variant,
  runtime-recursive-sum-by-spine-depth, list-pattern-in-variant, literal-mid-spine, sibling-two-sum, and any
  program that internally builds+matches compound sums: collatz, kernel/proof cases, host-RESULT lifts).
  All leak *reductions*, zero traps, zero value-wrong (v-runtime-verified on fresh samples + fence).
- **B-select (`reuse-clean` projection-operand refinement; commit ffa36a37a, verified):** the named
  simple-Option acceptance (`d4`/`dm1`/`d3`/`drs1`/`drs2`/`df2`) was NOT a separate emit path — it REACHES the
  same tail `MatchSum` gate but was over-declined by `reuse-clean`. Root: `expr_constructs_compound_seen`
  walked the arm's payload binders, which are inline `SumPayload{scrutinee}` reads, and descended into the
  scrutinee's OWN `(Some (list …))` `SumNew`/`ListNew` → a false-positive "the arm constructs a compound".
  Fix: `expr_constructs_compound_seen` must NOT descend into a borrowing-read's aggregate operand (`SumPayload`
  /`Proj`/`ListLen` — that operand is the borrowed scrutinee, not an arm construction); only flag constructors
  GENUINELY in the arm body. Reclaims **87 more cases** (every inline-constructed-scrutinee match), including
  the six pinned → 0. The `mts1`-style rebuild `(tuple (. t 0)(. t 1))` still declines (the `Tuple` is a
  top-level arm node, not a projection-operand). Zero traps / zero value-wrong across all 87 empirically
  settles the FBIP-miss question: `reuse-clean` was purely over-declining, never a soundness gate here.
- **Division of labor (why B-select is sound):** `escape-clean` guards ESCAPING payload aliases (a heap
  subvalue extracted and surviving the arm); `reuse-clean` guards REBUILD-into-surviving-result (a top-level
  arm constructor reusing shell cells). d4's list has scalar elements and the arm returns a scalar → nothing
  heap escapes → `escape-clean` correctly passes; the `reuse-clean` scrutinee-descent was a pure false
  positive. (CORRECTION to an earlier draft of this section, which wrongly claimed the simple family takes a
  `br_table`/`select` path that bypasses `emit_sum_cont` — it does not; it reaches the same gate.)

Remaining §B follow-ups: the extraction-retain release (inc2b, release-at-unwrap) and the handler-op-arg
owned reclaim (ap1 class — dup/drop the effect op-argument in the arm; see the ABI note below).

## Non-goals / discipline

- A leak beats a UAF. Until the analysis is complete, these families stay `(live-objects known-leak N)`
  (breaker banked them); the markers retire only when a witness provably reclaims.
- No incremental gate tweak that widens the *drop* without the matching *dup* on escaping payloads —
  twice it has trapped, and the Perceus reframe (above) identifies exactly why: those were missing-dup
  bugs, not should-not-drop cases. The fix is one place (the shared shell-reclaim/loop-back-edge
  decision) that emits the shell drop unconditionally AND guarantees every escaping bind is dup'd; the
  rc==1 runtime free is the safety net, FBIP reuse the fast path.

## Handler-op-arg owned reclaim (ABI note — the `ap1` class)

`ap1` (`put o st -> resume (match o ((Some (list a b)) (+ a b)) (_ -1))`) leaks its Option shell because the
`MatchSum` scrutinee `o` is a HANDLER OP-ARGUMENT (a `Core::Param`), which `heap_operand_ownership` classifies
Borrowed — so no one drops it. ABI ruling (v-runtime): **an effect op argument is owned-transferred to the
handler arm; the arm must drop it.** Rationale: the performer builds the arg and performs, which SUSPENDS —
its continuation receives the RESUME VALUE, never the argument back — so, unlike a normal borrowed-param call
(caller drops after return), there is no performer-side drop; the arm is the sole owner. Reporting `Owned` for
a handler-op-arg param makes `ap1` reclaimable.

Guards (v-effects, checked across the effect corpus): (1) multi-shot resume — fine (the arm owns the arg once
per perform; resume reuses the resume VALUE, not the arg). (2) non-resuming/abort arm — fine, and it must
STILL drop the arg it received. (3a) 🚩 an op-arg that IS or ALIASES the threaded heap STATE would be owned by
BOTH the state-thread and the arm-drop → DOUBLE-FREE; the `Param→Owned` flip must be NARROW (handler-op-arg
binder only, and EXCLUDE one that aliases the state slot) — never a blanket `Param→Owned` (that double-frees
every normal call). (3b) 🚩 a multi-use op-param → use-count-correct drop (retain per extra use, drop after
last), not a blind single drop. `ap1` itself is the clean shape (`o` and `st` distinct params, `o` matched
once, no alias) → sound; the general increment carries the 3a/3b guards.
