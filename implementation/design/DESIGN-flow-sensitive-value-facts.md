# Flow-sensitive value-facts for check elision — generalize the fact domain

**Author:** design (design-value-facts). **Status:** **PROPOSAL — scope DECIDED, stakeholder review
COMPLETE (all 6 owners signed off, 2026-07-21); READY FOR STAGE-1 VERTICAL.** Scope is confirmed
**(A) full generality, STAGED** (operator directive relayed via
pr-sync: "as general as possible so we can extend it to all data types" ⇒ all fact-kinds, sequenced;
slices stop-able after any increment). The operator is NOT iterating interactively — this doc is
written with concrete recommendations in place of every question, then circulated to the stakeholder
owners (v-core-opt, v-verification, v-wasm-opt, rust-backend, v-inference, v-compiler-perf) for review
(§7). This doc answers the operator's directive:

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
/// optional and independently join-able; `None`/absent = "unknown" (⊤, the safe default). A fact is a
/// conservative OVER-approximation of the value set (`actual ⊆ fact`); a check is elided only when the
/// fact PROVES it dead. So a too-WIDE fact (still ⊇ actual) is SAFE — it may fail to prove the check
/// dead, costing only a missed optimization; a too-NARROW fact (⊊ actual, dropping a real value) is
/// UNSOUND — it would wrongly prove the check dead and elide a needed guard. Therefore the join (at
/// control-flow merges) WIDENS (set-union of possibilities), the meet (at a refinement) NARROWS
/// (intersection to values the branch condition guarantees). (Corrected per PR#748 — the earlier draft
/// had this soundness direction reversed.)
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

**The load-bearing performance invariant (v-compiler-perf, confirmed 2026-07-21): the branch-frame
JOIN must stay per-var-O(1).** At a control-flow merge, each variable's `ValueFact` is joined by joining
each facet INDEPENDENTLY — `int_range` union, `nonzero` AND, `len_range` union, `variant_tags`
set-union — never any cross-variable work. This is exactly *why* D2 keeps a relational numeric domain
(`x < y` between two variables) OUT of the foundation: a relational facet would make the join O(vars²)
and turn a bounded constant-factor widening into a new algorithmic axis. With per-facet-independent
join, the widening is a bounded constant over today's interval cost (a few more `Option` fields to
clone/merge), NOT a new cost axis — verified against v-compiler-perf's linear-per-annotation descent
survey. A relational facet, if ever added, must carry its own cost justification; it is not free.

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
   behavior change — this is the safety floor the rest builds on). **LANDED (MR `51221170`, 2026-07-21).**
1b. **Flagship + reachability warning (§3.1, operator-directed).** Promote the already-live
   redundant-condition elimination (`tests.rs:13728`) to a headline corpus witness, AND add the
   reachability WARNING on a provably-dead branch (coordinate the CDZ code + wording with v-diagnostics).
   Pure integer-interval facts, so it sits right after slice 1. Gate: a warn-case pinning the emitted
   diagnostic on a fact-proven-dead branch + a companion no-warn case on an undecided condition.
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
   `if (< i (List.len xs))`. Add a Core-tier `provably_in_bounds(db, idx, coll, id) -> bool` predicate
   (sibling of `provably_no_overflow`). Consume at the indexed-access bounds check. Gate:
   provably-in-range index elides; out-of-range still traps. **Coordinate with v-runtime** (owns
   collection reps) on where the length is known at Core tier. **⚠ v-wasm-opt FLAG (2026-07-21): unlike
   the arith path, the wasm `List.at`/`Bytes.at` bounds guard is emitted INLINE** (`select.rs:2522/2775`
   = raw vec-len + compare + None/trap) and is NOT gated on any shared predicate — so this slice needs a
   one-line wasm arm change to consult `provably_in_bounds` before emitting the inline check, else the
   fact is bypassed on wasm. v-wasm-opt will do that wasm-side edit when the predicate lands (rust's
   bounds path already routes through Core predicates — v-rust-backend confirmed). This is the one
   emit-side change any slice requires; slices 1–3 (arith/nonzero) need zero backend change.
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

## 3.1 Operator refinement (relayed via pr-sync, 2026-07-21): flagship test + a reachability WARNING

The operator named a concrete demonstrator and a user-facing feature to fold in:

> "One good test of the value fact work is to show that redundant if checks are avoided. So like if we
> check `if x > 0`, anything inside the truthy branch can rely on that fact being true. And for example
> if there's another `if x > 0` then you just straight up always assume the truthy branch and remove the
> falsy branch entirely. It would actually be a good idea to emit a reachability warning there because
> it's just never going to be reached."

**(a) Flagship test — redundant-condition elimination: ALREADY IMPLEMENTED (the demonstrator, not new
work).** The exact case — a nested `(if (> n 0) …)` inside `(if (> n 0) …)`'s truthy branch collapsing
to only the taken branch with the dead branch deleted — already works today, the same finding as the
`x - 1` baseline (§0). It is driven by `refined_comparison_const` (`lower.rs:19367`) consumed at the
`Core::If` emit (wasm `select.rs:9147` "FLOW-SENSITIVE DEAD-BRANCH ELIMINATION"; the rust backend gets
the parity refinement push/pop via `refined_frame_for_branch`, `expr.rs:854`). It is GATED by
`a_branch_refinement_folds_a_redundant_nested_comparison_and_eliminates_its_dead_branch` (`tests.rs:13728`),
which pins, at the Lir level: the same-test (`> n 0` in `> n 0`), implied-test (`n >= 5 ⇒ n > 0`), and
made-false (`n < 0 ⇒ n > 10`) shapes — inner compare op gone, dead branch constant gone — plus value
parity and an over-folding guard (an undecided inner compare is NOT folded). **Action:** promote this to
a HEADLINE corpus witness of the value-facts feature (a `spec/semantics/NN-*.sexp` elision case), so the
"redundant checks avoided" the operator wants to SEE is a named, fleet-wide-gated demonstrator rather
than only a unit test. No new folding logic needed — the fact-propagation the operator describes is
exactly what's live.

**(b) NEW FEATURE — reachability WARNING on a provably-dead branch.** Beyond silently eliding the dead
branch, EMIT a diagnostic when the facts prove a branch never reached (the falsy arm of a
fact-proven-always-true condition, or vice-versa): "this branch is never reached — `<cond>` is always
`<true|false>` here (`<var> ∈ [lo, hi]`)". This is user-facing value (it surfaces dead code to the
programmer), distinct from the optimization. Design constraints:
- **A WARNING, not an error** — dead code is legal, just noteworthy.
- **Safety-critical, exactly as much as the elision:** fire ONLY when the interval facts PROVE the
  condition constant (the same `refined_comparison_const`-decided predicate that licenses the fold). A
  false "always true" that warns-and-removes a REACHABLE branch would be a miscompile, so the warning is
  gated on the identical conservative proof — never on a heuristic.
- **Where it fires:** at the fact-analysis site that already decides the branch (the `Core::If` dead-
  branch elimination). The diagnostic is produced from the analysis, then routed through the standard
  diagnostic surface.
- **Diagnostic code + wording: OWNED BY v-diagnostics** (they own the actionable-diagnostic surface +
  CDZ codes). Coordinated via `note` (2026-07-21): whether a NEW CDZ code (a CDZ04xx "unreachable
  branch") or an existing reachability/dead-code code fits, and the exact phrasing, is their call; the
  fact-analysis supplies the fact (`<var> ∈ [lo,hi]`, which arm is dead) and they own the message shape.
- **A natural early slice:** the redundant-condition case is pure integer-interval facts (stage-1
  territory), so the warning lands as an early slice (after slice 1's refactor) rather than waiting on
  the non-integer facets. Gate: a reject/warn corpus case pinning the emitted warning on a
  provably-dead branch AND a companion case where an UNDECIDED condition emits NO warning (the
  over-warning guard, mirroring the over-folding guard already in `tests.rs:13728`).

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

**The WIN metric (v-compiler-perf, confirmed 2026-07-21): measure DETERMINISTIC emitted-guard COUNT,
never wall-clock** (an A/B wall-clock delta is below the fleet-load noise floor). Each slice gates on a
guard-count *reduction*, three tiers:
- **(a) Elision witness** — a program where the fact licenses dropping a guard; assert the drop
  STRUCTURALLY, preferably by pinning the exact emitted-instruction count (guarded form emits N instrs,
  fact-elided form N−k, pinned in an rcdzc select/emit unit test), or by a guard-presence probe
  (`wasm-tools print | grep <guard-shape>` — the trap block / div-by-zero check / bounds compare is
  ABSENT for the elided case, PRESENT for the control).
- **(b) Soundness twin** — the paired case with the fact-establishing check REMOVED that MUST still
  trap/misvalue (the §4b differential — proves the elision is fact-licensed, not luck).
- **(c) Corpus-wide guard-count baseline** (standing aggregate) — the sum of overflow/div/bounds guard
  blocks emitted across the behavior corpus, committed as a number each slice DRIVES DOWN; a peer change
  that spuriously re-introduces guards makes it go UP = a visible regression. v-compiler-perf offered to
  build this counter + wire it into `xtask` (the emit-side analogue of their alloc-bench). The per-slice
  (a)+(b) pins are the minimum; (c) is the optional aggregate view.

---

## 4.1 The three-disjunct elision seam (agreed with v-verification + v-core-opt)

The single both-backend elision decision is already a disjunction, and this design adds the fact-path as
an **independently-sound third disjunct** — it does NOT collide with v-verification's parallel b3 work.
Both efforts target `lower::provably_no_overflow` (`lower.rs:19760`). The agreed composition:

```rust
// The union: elide the overflow guard IFF ANY disjunct independently proves it dead.
// Each disjunct is FAIL-CLOSED (returns false = "this path can't prove it", never unsound) and
// INDEPENDENTLY SOUND (each alone licenses elision; the OR only ever elides MORE, only on a real
// fact/proof). Ownership is partitioned so no two efforts edit the same function body.
provably_no_overflow(db, op, lhs, rhs, ty, id) =
      arith_provably_in_range(db, op, lhs, rhs, ty)   // interval analysis — EXISTS today (v-core-opt seam)
   || fact_proven_safe(db, op, lhs, rhs, ty, id)      // THIS design's fact-path — NEW disjunct (I own)
   || discharged_no_overflow(db, id)                  // LCF-kernel proof — v-verification's b3 (they own)
```

- **Disjunct ownership (the anti-collision contract):**
  - `arith_provably_in_range` — v-core-opt (the existing interval seam). Untouched by this design *until*
    slice 1's refactor, which is a pure port to the `ValueFact::int_range` facet, coordinated with them.
  - `fact_proven_safe(db, op, lhs, rhs, ty, id) -> bool` — **THIS design's new disjunct.** A peer function
    added alongside the other two, consulting the `ValueFact` environment. During slices 1–2 its integer
    logic may simply *be* the generalized `arith_provably_in_range` (they merge); the separate name is the
    stable seam for the non-integer facets (nonzero/len/tag reuse the same fail-closed shape at their own
    consumers, e.g. `provably_nonzero_divisor`).
  - `discharged_no_overflow(db, id) -> bool` — **v-verification's b3, they own it exclusively.** I do NOT
    edit it; they do NOT edit `fact_proven_safe` or `arith_provably_in_range`.
- **v-core-opt owns the disjunction WRAPPER** (`provably_no_overflow` itself / the Slice-5 seam). Any new
  disjunct is added there by agreement, so the three efforts touch three different function bodies + one
  jointly-reviewed wrapper.
- **Why this is sound as a union:** disjunction of independently-sound predicates is sound — a guard is
  elided only when at least one disjunct *proves* it dead; a disjunct that can't prove it returns false
  and the others (or the kept check) still cover the input. No disjunct can *force* a guard to stay, and
  none can elide a guard a real input needs. This is the same fail-closed argument each disjunct satisfies
  alone.
- **Signature confirmed to v-verification** (their `note` asked for it): my disjunct is
  `fn fact_proven_safe(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId, result: IntTy, id: StructId)
  -> bool`, placed as a peer disjunct inside `provably_no_overflow`, added by v-core-opt in the wrapper.
  They land their discharge plumbing first and flip their optimizer last; I land slice 1 (the refactor)
  without touching their disjunct.
- **CONFIRMED by both seam owners (2026-07-21):** v-verification confirmed §4.1/§5 and noted their
  `discharged_no_overflow` **stays a `false` stub** for now (b3 is operator-(A)-blocked — needs the b4
  per-node channel + a fuel-bounded compile-time kernel interpreter rcdzc lacks), so there is **no
  near-term OR collision from their arm** and slice 1 can land freely. v-core-opt confirmed the wrapper
  is the right place and the shape matches what they already approved. **Wrapper-edit ORDER (v-core-opt
  directive):** the wrapper body (`provably_no_overflow`, `lower.rs:~19812`) must not be edited by two
  efforts the same tick — whoever adds their OR-term first wins, the next rebases onto it; v-core-opt
  offered to arbitrate / do the single wrapper edit. Since v-verification's disjunct is dormant, this
  design's slice adding `fact_proven_safe` is the next wrapper edit — coordinate the one-line change with
  v-core-opt when slice 1/2 lands it.

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

## 6. Decisions (all resolved — no operator round-trip needed)

- **D1 — Primary axis / how far to go. DECIDED: (A) full generality, STAGED** (operator directive
  relayed via pr-sync, 2026-07-21). The operator's own words — "as general as possible so we can extend
  it to all data types" — are option (A). Build all fact-kinds, sequenced (§3), with slices **stop-able
  after any increment** (if priorities shift, bank the shipped slices and pause — no all-or-nothing
  risk). Alternatives (B) integers-only and (C) minimal-CorePass-only were rejected as contradicting the
  stated "all data types" intent.
- **D2 — Fact shape. DECIDED: the product-of-facets `ValueFact`** (§2), NOT a full relational
  abstract-interpretation numeric domain (octagons/polyhedra) up front. Rationale: the extensible
  known-predicate set gives the operator's "all data types" generality (add a facet = add a lattice
  element + its transfer functions) at a fraction of the cost; relational `x<y` is a later ADD-ON facet,
  not the foundation. Noted here as the considered alternative per the mode-change instruction.
- **D3 — Soundness posture. DECIDED: conservative + per-slice differential gate + (from slice 3) a
  fuzzer differential mode.** No aggressive/speculative elision; anything the facts can't prove stays a
  job for v-verification's kernel arm (the third disjunct, §4.1).
- **D4 — Where it runs. DECIDED: slices 1–5 in the existing emit-time refinement machinery** (the
  operator's "extend the core lowering" — this IS in the Core lowering), and only slice 6 promotes to a
  materialized `CorePass`. This lets value land WITHOUT waiting on the pass-framework migration.

The design is buildable slice-by-slice with zero regression risk at every step. No decision is left open
for the operator; any genuine fork surfaced by stakeholder review that the owners can't resolve is
escalated to pr-sync (not the interactive widget).

---

## 7. Stakeholder review — COMPLETE, all owners signed off (2026-07-21)

Per the operator's mode-change (relayed via pr-sync: "have it write up a proposal and get the other
stakeholders to review it"), the doc was circulated to the owners below and **all six responded and
confirmed**. Their concrete feedback is folded into §2 / §3 / §4 / §4.1 above.

| Owner            | Verdict & folded-in feedback                                                             |
|------------------|------------------------------------------------------------------------------------------|
| **v-verification** | **CONFIRMED §4.1/§5** three-disjunct seam + `fact_proven_safe` signature. Their `discharged_no_overflow` **stays a `false` stub** (b3 operator-(A)-blocked) → no near-term OR collision; slice 1 lands freely. (§4.1) |
| **v-core-opt**   | **CONFIRMED** the wrapper is the right place + the shape matches their prior agreement; slice-6 O1(int/nonzero)/O2(len/tag) tiers **CONFIRMED correct**; `--opt-sweep` level-equivalence is the mandatory gate. Directive: **coordinate the one-line wrapper edit order** (don't co-edit `lower.rs:~19812`). (§4.1) |
| **v-rust-backend** | **CONFIRMED** — audited every rust arith/div/shift guard; NONE range-checks independently of the shared Core predicate. New fact-kinds elide with **zero rust change**. Will re-audit when a new predicate lands. |
| **v-wasm-opt**   | **CONFIRMED** arith/shift = zero wasm change (predicate-routed). **🚩 FLAG folded into slice 4:** `List.at`/`Bytes.at` bounds guard is INLINE (`select.rs:2522/2775`), not predicate-routed → the `len`→bounds facet needs a one-line wasm arm change to consult `provably_in_bounds`; they'll do it wasm-side. |
| **v-inference**  | (Note delivered; reads `resolved_int_bounds` as the width source, `variant_tags` orthogonal to their match width-narrowing.) |
| **v-compiler-perf** | **CONFIRMED** cost OK **iff the join stays per-var-O(1)** (validates D2 — keep relational facets out; folded into §2). WIN METRIC = **deterministic emitted-guard COUNT**, not wall-clock: per-slice (a) elision witness + (b) soundness twin + optional (c) corpus-wide guard-count baseline they'll build. (folded into §4) |

**Convergence → handoff:** both load-bearing sign-offs (v-verification + v-core-opt on §4.1) are IN, so a
**stage-1 vertical-ready brief** goes to the PM (`design-flow-sensitive-value-facts.md` in the queue),
pointing at this doc, naming subsystem `rcdzc`, and scoping the first increment as **slice 1 (the
`ValueFact` interval-only refactor — behavior-identical, the safety floor)**. Later slices are follow-on
increments the vertical carries top-to-bottom.

---

## 8. Implementation outcome (as-built, 2026-07-22 — §0–§7 above are the original proposal)

The `v-value-facts` vertical landed the following. §0–§7 are the design-of-record; this section is the
honest as-built record, including where reality diverged from the plan.

**LANDED + fleet-gated (12 corpus witnesses + 7 rcdzc unit tests):**
- **Slice 1** — `ValueFact` interval-only refactor (behavior-identical). ✅
- **Flagship (§3.1a)** — the redundant-`if` elimination was found ALREADY LIVE (`refined_comparison_const`
  at the `Core::If` emit); promoted to a headline corpus witness. No new folding needed.
- **Slice 2 (integer-facet gaps)** — UNSIGNED comparison refinement (vs a non-negative constant, no
  wraparound). ✅ **Caught + fixed a UInt64 soundness bug pre-land**: seeding the interval from a hardcoded
  `(i64::MIN, i64::MAX)` fabricated a false ceiling for a UInt64 (whose real hi is `None`); fixed to seed
  from `resolved_int_bounds`. Pinned with a UInt64-ceiling soundness twin + a domain-edge (0-lower-bound
  tautology) witness.
- **`fact_proven_safe` disjunct (§4.1)** — the third independently-sound disjunct, landed behavior-neutral
  (delegates to `arith_provably_in_range` for the integer facet; the named seam for later facets). ✅
- **Slice 6c (NEW, beyond the original plan)** — a flow refinement folds a comparison over a checked-arith
  operand (`(< (+ x 1) 11)` under `x ∈ [0,9]`) when the arith is **provably-no-overflow** (a
  trap-free-in-context check reusing the overflow oracle — discarding a checked arith is trap-safe only
  when it can't overflow). Lir + corpus pinned, incl. an overflow-capable soundness twin (must NOT fold).

**DIVERGED from the plan:**
- **Slice 3 (`nonzero` / div-by-zero)** — DEPRIORITIZED. Finding: the div-by-zero guard is EXPLICIT only on
  the rust backend; wasm uses the machine's native `div_s`/`div_u` ÷0 hardware trap (no emitted guard to
  elide). So a `nonzero` fact elides only rust-side — a minor single-backend win. Separately, the signed
  `MIN/-1` div-overflow elision ALREADY benefits from flow refinement on BOTH backends (`value_provably_nonneg`
  → `value_range` → `range_refinements`), pinned at the Lir level.
- **Slice 5 (`variant_tags` / redundant-match elision)** — the FACET (a disjoint `variant_refinements` stack
  + `refined_variant`) was built and is correct, and v-patterns wired an emit consult — but it turned out
  the same-scrutinee nested-match elision is ALREADY delivered by `lower_match_sum`'s existing `known_disc`
  const fold (two tiers up in resolve), so the emit consult fired 0× (dead code). Both the emit consult and
  the facet were **removed** (behavior-neutral). Net: the sum-facet elision works, via the pre-existing
  lower fold, not new value-facts code. TWO durable insights survived: (i) a latent **use-after-free class**
  (a mid-emit `core_of` on a call scrutinee β-reduces/inlines and renumbers a payload out of `dup_sites` →
  dropped retain; the fix is a `core_of`-free `resolved_of` consult), and (ii) a compile-time **~O(N^1.5)
  superlinearity** in `lower_match_sum`'s per-level `const_at_path` walk on deep same-scrutinee nests (a
  `lower_match_sum` concern, memoizable — v-patterns' territory).
- **Slice 4 (`len_range` / collection-bounds)** — DEFERRED. Finding: the canonical `(if (< i (List.len xs))
  (List.at xs i) …)` bounds pattern is RELATIONAL (index vs a runtime length — two runtime values), which
  §2/D2 deliberately excludes from the foundation. A constant-list + constant-index `List.at` already folds
  in `lower`. So a non-relational `len_range` facet covers only a thin residual (runtime index into a
  const-length list, const-bounded) — its real payoff is gated on the relational facet below.

**PENDING (operator-gated):**
- **Relational facet (design (B) / §2 "later ADD-ON")** — the high-value unlock (both `(< i len)` bounds
  elision AND general `x<y` overflow elision). Operator deferred it as its own future slice ("the real prize
  later, greenlight on request") because a FULL relational domain is O(vars²)-join. Open question routed to
  v-compiler-perf: can a BOUNDED relational facet (fixed-cap pair set / index-vs-length-only / known-below
  map) stay per-var-cheap enough to pass the join guardrail? Awaiting their cost read + the operator greenlight.
- **Reachability WARNING (§3.1b, CDZ0308)** — v-diagnostics landed the CDZ0308 scaffold; the emit is
  fork-blocked (the dead branch is folded away before a post-hoc pass can see it → capture must be at the
  fold site). Routed to the operator (concierge) as a follow-on; the flagship elision itself ships regardless.

**Net:** the operator's "as general as possible / all data types" directive is delivered for the shipped
surface — the integer facet is complete and thoroughly gated, and the sum-facet elision is live (via the
pre-existing lower fold). Every landed slice is soundness-gated with a differential twin, and the arc caught
and fixed two real miscompile classes (the UInt64 ceiling and the slice-5 latent UAF). The remaining
high-value increment is the relational facet, pending the cost read + operator greenlight.
