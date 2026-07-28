# Vertical-ready: flow-sensitive value-facts for check elision (stage 1)

**Spec:** `implementation/design/DESIGN-flow-sensitive-value-facts.md` (PROPOSAL, scope DECIDED,
stakeholder review COMPLETE — all 6 owners signed off 2026-07-21).
**Subsystem:** `rcdzc` (Rust compiler; `implementation/seed/crates/rcdzc/src/`).
**Scope:** operator-directed **(A) full generality, STAGED** — a pluggable `ValueFact` lattice
generalizing the existing signed-interval overflow-elision analysis to all data types + check kinds.
Slices are **stop-able after any increment** (no all-or-nothing).

## Why this is buildable now
The operator's motivating example (`if x > 0` ⇒ `x - 1` can't underflow) ALREADY elides today — rcdzc
has a live flow-sensitive interval analysis (`value_range`, `range_refinements`, `provably_no_overflow`,
`refine_from_comparison`). This is a GENERALIZATION, not greenfield. The elision seam is a 3-disjunct
`OR` all owners have confirmed (§4.1 of the doc). Stage 1 is a **behavior-identical refactor** — the
safest possible first vertical.

## First increment (slice 1) — the concrete task
Port the fact representation from the bare tuple to a `ValueFact` struct, WITHOUT changing behavior:
- Introduce `ValueFact { int_range, nonzero, len_range, variant_tags }` (only `int_range` populated in
  slice 1). See doc §2 for the exact shape.
- Widen `Db::range_refinements` from `FxHashMap<StructId, (i64, Option<i64>)>` to
  `FxHashMap<StructId, ValueFact>`; `value_range` / `refine_from_comparison` / `refined_frame_for_branch`
  produce+consume the `int_range` facet.
- **LOAD-BEARING invariant (v-compiler-perf):** the branch-frame JOIN must stay per-var-O(1) — join each
  facet independently, never cross-variable work. This is why a relational domain is OUT of the
  foundation (doc §2).
- **Gate (v-compiler-perf + v-core-opt):** full corpus + `xtask gate` byte-identical at O0..O2 on BOTH
  backends (zero behavior change — this is the safety floor). No wrapper edit yet (slice 1 is the
  refactor; `fact_proven_safe` disjunct lands slice 2).

## Follow-on increments (the vertical carries top-to-bottom; see doc §3)
- **Slice 2:** fill integer-facet gaps (unsigned comparisons, `Eq`/`Ne`) + add the `fact_proven_safe`
  disjunct to `provably_no_overflow` (`lower.rs:~19812`). **⚠ COORDINATE the 1-line wrapper edit with
  v-core-opt** (don't co-edit with v-verification's dormant disjunct — v-core-opt arbitrates order).
- **Slice 3:** `nonzero` facet + div-by-zero elision (new `provably_nonzero_divisor` predicate).
- **Slice 4:** `len_range` facet + collection-bounds elision. **⚠ v-wasm-opt FLAG:** `List.at`/`Bytes.at`
  bounds guard is INLINE (`select.rs:2522/2775`), needs a 1-line wasm arm to consult `provably_in_bounds`
  — v-wasm-opt does the wasm-side edit. Coordinate v-runtime (collection reps).
- **Slice 5:** `variant_tags` facet + redundant-match-arm/discriminant elision (the general endpoint).
  Coordinate v-patterns.
- **Slice 6 (capstone):** materialize facts as a registered `ValueFactPass` CorePass in `opt.rs`
  (O1 int/nonzero, O2 len/tag), gated by `--opt-sweep` level-equivalence. **⚠ Consult v-core-opt +
  v-verification before landing** (shared-fact-column synergy with the kernel proof path).

## Win metric (v-compiler-perf, per slice)
Deterministic emitted-guard COUNT, NOT wall-clock: (a) elision witness (pin reduced instr count /
guard-presence probe), (b) soundness twin (fact-establishing check removed ⇒ must still trap), optional
(c) corpus-wide guard-count baseline v-compiler-perf will build.

## Suggested owner
A `vertical` agent, area=`rcdzc` (or a compiler-opt-focused vertical). Stage-1 is self-contained in
`lower.rs`/`diverge.rs`/`db.rs` and needs no cross-territory hand-off; later slices coordinate the owners
named above.
