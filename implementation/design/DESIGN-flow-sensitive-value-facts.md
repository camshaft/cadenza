# Flow-sensitive value-facts for check elision — generalize the fact domain

**Author:** design (design-value-facts). **Status:** **SCOPING (operator idea via concierge/pr-sync,
2026-07-21).** This doc answers the operator's directive:

> "On overflow checks we should really be able to take into account known facts about an integer from
> previous checks. So if I say `if x > 0` and I'm in the truthy branch we should be able to safely
> `sub 1` without any risk of overflow. So we should really extend the core lowering to build facts
> about values and then take that into account. And it would be great to be as general as possible so
> we can extend it to all data types."

The important finding up front: **the operator's exact example ALREADY WORKS today.** rcdzc has a real
flow-sensitive interval analysis that refines `x` to `[1, MAX]` in the truthy branch of `if x > 0` and
elides the `x - 1` underflow guard on BOTH backends. This doc is therefore NOT greenfield. Its value is
the *generalization axis* the operator explicitly asked for — lifting a signed-integer-interval-only,
emit-only analysis into a **general, pluggable value-fact lattice** extensible to all data types and all
check kinds — and closing the concrete gaps in what exists. Line numbers are landmarks at this commit,
not promises they won't drift.

---

## 0. What already exists (the honest baseline — measure before building)

A flow-sensitive interval/range dataflow analysis is live in `rcdzc` and already drives overflow-guard
elision on both backends. The pieces (all `implementation/seed/crates/rcdzc/src/`):

- **The fact domain today = a signed integer interval** `(i64, Option<i64>)` (`hi = None` ⇒ unbounded
  above). There is no named `Interval`/`Fact` type — it's this tuple threaded everywhere.
- **`value_range(db, id) -> Option<(i64, Option<i64>)>`** (`lower.rs:19432`) — the core query. Sources:
  constant → `[v,v]`; a flow-sensitive refinement (below) intersected with declared-type bounds;
  `let`-initializer propagation; arithmetic propagation (`arith_range`, `lower.rs:19554`, all interval
  math in `i128`); conditional/match branch **union**; collection lengths → `[0, 2^32−1]`; else the
  declared integer type's bounds.
- **Flow-sensitivity is already present** — an EMIT-ONLY refinement stack:
  - `Db::range_refinements: Vec<FxHashMap<StructId, (i64, Option<i64>)>>` (`db.rs:~1878`) — a stack of
    frames, transient, NEVER memoized (so a branch-local fact can't poison a cached range).
  - `push_range_refinements` / `pop_range_refinements` / `refined_range` / `current_refinements`
    (`db.rs:2730..2752`), bracketed around each `if` arm during emit.
  - `refined_frame_for_branch` (`backend/common/diverge.rs:88`) + `refine_from_comparison`
    (`diverge.rs:125`) compute the interval a branch condition GUARANTEES: `(op var C)` narrows `var`
    in the taken branch (negated in the other), `if (= x c)` pins `[c,c]`, and it composes through
    `and`/`or`/`not`. This is exactly "learn a fact from a previous check."
- **The elision decision is UNIFIED across backends**:
  `provably_no_overflow(db, op, lhs, rhs, ty, id) = arith_provably_in_range(...) OR
  discharged_no_overflow(db, id)` (`lower.rs:19760`). Consulted by wasm `emit_checked_arith_to`
  (`backend/wasm/select.rs:~14725`) AND rust `emit_arith` (`backend/rust/expr.rs:3183`). Companion
  fact-consumers already exist: modulo elision (`lower.rs:17479`), `[0,2^B)` narrowing
  (`lower.rs:17487`), `shl_provably_in_range` (`lower.rs:19777`), divisor-can-be-`-1`
  (`lower.rs:19870`), comparison folding (`lower.rs:19313`/`19349`).
- **The proof seam is stubbed**: `discharged_no_overflow` (`lower.rs:19749`) is a `false` stub — the
  hook where an LCF-kernel proof licenses an elision the intervals can't prove. **This half is OWNED BY
  v-verification** (see `DESIGN-verification-program-conditions.md` §3, "b3/b4"). This design does NOT
  touch it — see §5 (Territory).
- **The pass framework exists**: `opt.rs` has `CorePass` + `PassManager` + `OptLevel` (O0..O3, default
  O1) and a prototype `ProofElisionPass`, but registers no passes yet — today facts are re-derived
  lazily at emit time, not materialized once. See `DESIGN-tiered-optimization-levels-rcdzc.md`.

**Net:** the operator's stated example is a solved case. The unsolved thing is *generality*.

---

## 1. The gap — what the operator's "as general as possible / all data types" is really asking for

The current analysis is narrow on FOUR concrete axes. Each is an increment below.

1. **Fact SHAPE is a signed integer interval only.** `refine_from_comparison` SKIPS unsigned
   comparisons and `Eq`/`Ne` in the else-branch (`diverge.rs` header is explicit), and there is no
   representation for: nonzero, exact-sign, relational facts (`x < y` between two *variables*, not
   `x < const`), collection-length-of-*this*-value, or "this sum value is provably variant K here."
   The operator's "all data types" lands here: an integer interval says nothing about a `List`, a
   `String`, or a user sum.
2. **Fact SOURCES are `if`-comparison-vs-constant only.** Facts learned from a `match` arm (the
   scrutinee IS variant K, a bound field has a known tag), from a prior *arithmetic* guard, from an
   already-checked bounds access, or from a `@requires` precondition, are not propagated as reusable
   facts (match-arm exact-value refinement exists narrowly for `if (= x c)` but not for constructor
   patterns).
3. **CHECK KINDS elided are overflow/underflow (+shift/mod/narrow) only.** Div-by-zero, collection
   bounds, and narrowing conversions are each checked independently and do not consult a shared
   nonzero/length fact.
4. **Facts are EMIT-ONLY and re-derived per backend call.** Nothing materializes the fact set as a
   queryable column, so (a) v-verification can't share it, and (b) it's recomputed at each emit site.

The design is: **introduce one general `ValueFact` lattice and a flow-sensitive fact environment that
subsumes the current interval stack, then widen sources and check-consumers on top of it — each slice
conservative, differential-gated, behavior-neutral.**

---

## 2. The fact lattice (the core design decision — RECOMMENDED shape)

A `ValueFact` is a conservative OVER-approximation of the set of values a `StructId` occurrence may
take in the current flow context. The lattice is a PRODUCT of independent, individually-optional facets
so it stays extensible ("add a facet" ≠ "rewrite the domain") — the operator's generality goal:

```rust
/// A conservative fact about the value at a Core occurrence, in the current flow context. Every field is
/// optional and independently join-able; `None`/absent = "unknown" (⊤, the safe default). A fact only ever
/// NARROWS the true value set — a wrong-because-too-wide fact is unsound, a too-narrow one is just missed
/// optimization. So the join (at control-flow merges) WIDENS (set-union of possibilities), the meet (at a
/// refinement) NARROWS (intersection).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ValueFact {
    /// Signed integer interval [lo, hi] — SUBSUMES today's `(i64, Option<i64>)`. `None` = unbounded that side.
    pub int_range: Option<(Option<i64>, Option<i64>)>,
    /// Provably != 0 (elides div-by-zero). Cheap, high-value; derivable from a range excluding 0 but also
    /// established directly (`if x != 0`, a checked divisor).
    pub nonzero: bool,
    /// For a collection/string value: known length interval (elides bounds checks on provably-in-range index).
    pub len_range: Option<(u64, Option<u64>)>,
    /// For a sum-typed value: the set of variant tags it may be here (a singleton elides a match discriminant
    /// / redundant arm). `None` = any variant. This is the "all data types" generality — works on ANY sum.
    pub variant_tags: Option<VariantSet>,
}
```

Rationale for a product-of-facets over the two obvious alternatives:
- **vs. a full abstract-interpretation numeric domain (octagons/polyhedra for relational `x<y`):**
  rejected as the STARTING point — superlinear cost, huge surface, and the payoff (relational facts) is
  a small fraction of real elisions. A single relational facet can be ADDED later (a `rel: Vec<(StructId,
  Cmp, StructId)>` facet) without disturbing the rest; we do not pay for it up front.
- **vs. staying with the bare interval tuple:** rejected because it structurally cannot express
  nonzero/length/tag — the operator's "all data types." The product type is the minimum general shape.

The **flow-sensitive fact environment** generalizes today's `range_refinements` stack from
`FxHashMap<StructId, (i64,Option<i64>)>` to `FxHashMap<StructId, ValueFact>`. The push/pop bracket, the
emit-only/never-memoized discipline, and the branch-frame merge all carry over unchanged — this is a
type-widening of an existing, proven mechanism, not a new subsystem.

**Soundness invariant (load-bearing, restated):** a fact may only be ADDED when the analysis can prove
it holds on the branch. The elision consumer ALWAYS defaults to keeping the check when the fact is
absent/⊤. So a bug in fact *derivation* that produces too-wide a fact is the only way to miscompile —
and every increment is gated by a differential corpus case that WOULD trap/miscompute if the fact were
wrong, on both backends (§4).

---

## 3. Increments (top-to-bottom, the way a vertical lands them)

Each slice is independently green, conservative, and differential-gated. Ordered so the earliest slices
are pure refactors (behavior-identical) and value arrives without ever risking a regression.

1. **Introduce `ValueFact` as an interval-only wrapper; port `value_range` + `range_refinements` to it.**
   Pure refactor: `ValueFact { int_range: .., ..Default }`, the environment becomes
   `FxHashMap<StructId, ValueFact>`, `refined_range`/`refine_from_comparison` produce/consume the
   `int_range` facet. **Gate: full corpus + gate byte-identical at O0..O2, both backends** (zero
   behavior change — this is the safety floor the rest builds on).
2. **Fill the integer-facet gaps** the current analysis skips: unsigned comparisons and `Eq`/`Ne`
   two-sided refinement where sound. Gate: a differential case where an unsigned `if x < 8` then a
   masked/indexed use elides on both backends AND a case that must STILL trap.
3. **`nonzero` facet + div-by-zero elision.** Establish `nonzero` from `if x != 0`, from a range
   excluding 0, and from a prior checked division. Consume it at the div/rem guard (a new
   `provably_nonzero_divisor(db, id)` consulted by both backends' div emit, mirroring
   `provably_no_overflow`). Gate: `(if (!= d 0) (/ n d) …)` elides its zero-check; `(/ n d)` unguarded
   still traps.
4. **`len_range` facet + collection-bounds elision.** Length facts already exist as `[0, 2^32−1]`;
   promote to a real facet sourced from `List.len`/literal construction and refined by
   `if (< i (List.len xs))`. Consume at the indexed-access bounds check. Gate: provably-in-range index
   elides; out-of-range still traps. **Coordinate with v-runtime** (owns collection reps) on where the
   length is known at Core tier.
5. **`variant_tags` facet + redundant-discriminant/arm elision** — the most "all data types" slice.
   Source: a `match` arm binds the scrutinee to variant K (so inside the arm its tag set is `{K}`); a
   prior `is-K?` guard. Consume: a nested match/discriminant test on a value with a singleton tag set
   folds to the known arm. Gate: nested match on an already-refined sum drops the inner discriminant;
   behavior unchanged. **Coordinate with v-patterns** (owns match lowering).
6. **Materialize as a registered `CorePass`** (optional capstone) — lift the emit-time fact derivation
   into a fact column computed once by a `ValueFactPass` (`opt.rs` `CorePass`, `min_level` O1 for the
   cheap facets / O2 for anything whole-function), so both backends AND v-verification's discharge share
   ONE fact set. Gate: the `--opt-sweep` level-equivalence gate (`DESIGN-tiered-optimization-levels`
   §5). **Coordinate with v-core-opt** (owns `opt.rs`).

Slices 1–2 are the "close the integer gaps" core; 3–5 are the "all data types" generalization; 6 is the
sharing/perf capstone. A vertical can stop after any slice with a coherent, gated surface.

---

## 4. Soundness & the gate (the correctness bar — non-negotiable)

Eliding a check must NEVER drop a real trap. The discipline, matching the fleet's existing conservative
posture and the tiered-opt level-equivalence rule:

- **Conservative by construction:** every consumer defaults to KEEPING the check; a fact only ever
  removes a check the fact proves dead. Absent/⊤ fact ⇒ status quo.
- **Per-slice differential gate:** each increment lands with (a) a corpus case that elides (proving the
  win) AND (b) a paired case with the guard-establishing check REMOVED that MUST still trap/misvalue —
  proving the elision is licensed by the *fact*, not by luck. Both backends (`gate` + `gate --target
  rust`).
- **Level equivalence:** once slice 6 tiers facets by `OptLevel`, the `xtask gate --opt-sweep`
  (`DESIGN-tiered-optimization-levels` §5) proves O0..O3 compute the identical value/trap — a level that
  changes a result is a hard fail.
- **Fuzzer hook:** ask v-fuzzer for a differential mode that compiles a program with facts ON vs. a
  facts-OFF reference and diffs the observable result — a divergence is an unsound elision. (This is the
  strongest guard for the general lattice; propose after slice 3.)

---

## 5. Territory & coordination (who owns what — avoid collision)

This design deliberately occupies the **interval/fact-domain generalization on the non-proof side**. The
adjacent seams and their owners:

- **v-verification owns `discharged_no_overflow` (the LCF-kernel proof path, b3/b4)** —
  `DESIGN-verification-program-conditions.md` §3. Our facts and their proofs meet at the SAME consumer
  (`provably_no_overflow`'s `OR`), but we do not fill their stub and they do not touch our lattice. The
  natural synergy: a materialized fact column (slice 6) is a sound, cheap SOURCE the kernel can cite;
  conversely a discharged `Thm` is a fact our lattice can't derive. Keep them as the two arms of the
  existing `OR`. **Consult before slice 6.**
- **v-core-opt owns `opt.rs` (the `CorePass`/`PassManager`/`OptLevel` framework).** Slice 6 registers a
  pass under their framework — coordinate the tier assignment and pass ordering. **Consult before slice
  6.**
- **v-compiler-perf** — this is fundamentally their hot path (elision = fewer emitted guards). Loop them
  on the perf probe / emit-size measurement.
- **v-inference** — owns width facts (`literal_width_fault`, `infer.rs:768`). Integer facets should
  read declared width bounds via their `resolved_int_bounds` rather than re-deriving.
- **v-runtime** (slice 4, collection lengths) and **v-patterns** (slice 5, match/variant tags) — the
  respective data-type owners; the fact SOURCE for those facets lives in their lowering.

Rule: `note` the owner and agree the seam BEFORE a cross-territory slice; slices 1–3 are entirely within
the existing `lower.rs`/`diverge.rs`/`db.rs` range machinery and need no hand-off.

---

## 6. Open decisions (chosen defaults — the operator can override any)

- **D1 — Primary axis / how far to go.** DEFAULT: build the generalization (slices 1–5), because the
  operator explicitly asked for "as general as possible / all data types" and the plain-interval case
  already works. If the operator only wants "more integer checks elided," slices 1–3 suffice and 4–5
  drop. **Routed to the operator as an `ask` (this is the one genuinely-forking scope call).**
- **D2 — Fact shape.** DEFAULT: the product-of-facets `ValueFact` (§2), NOT a full relational numeric
  domain up front. Relational `x<y` is a later ADD-ON facet, not the foundation.
- **D3 — Soundness posture.** DEFAULT: conservative + per-slice differential gate + (from slice 3) a
  fuzzer differential mode. No aggressive/speculative elision; anything the facts can't prove stays a
  job for v-verification's kernel arm.
- **D4 — Where it runs.** DEFAULT: keep slices 1–5 in the existing emit-time refinement machinery (the
  operator's "extend the core lowering" — this IS in the Core lowering), and only slice 6 promotes to a
  materialized `CorePass`. This lets value land WITHOUT waiting on the pass-framework migration.

The design is buildable slice-by-slice with zero regression risk at every step; the only decision that
gates scope (not safety) is D1, which is with the operator.
