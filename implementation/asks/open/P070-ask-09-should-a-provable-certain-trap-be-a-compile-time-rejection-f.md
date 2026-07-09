## 9. 🔴 Should a provable-certain trap be a compile-time rejection? (fold stays meaning-preserving either way)

**Finding.** The Cadenza compiler's `fold` pass guards division/modulo folding with `foldable-divisor`:
it collapses `(/ c d)` / `(% c d)` to a constant ONLY when the divisor is a non-zero, non-overflowing
constant; otherwise it leaves the primitive in place so the trap happens at run time as written. This
is the correct floor for the *fold* — an optimizer must preserve meaning, and `(/ 5 0)`'s recorded
meaning today is `(trap "division by zero")` (a trap is a legitimate terminal condition, constitution
§"exactly one terminal condition"), so folding it to a value would be a miscompile. Writing the guard
raised a policy question the operator flagged: **if we can PROVE at compile time that a program will
unconditionally trap, shouldn't we REJECT it at compile time rather than ship a program that traps?**

**Why it touches the spec.** This changes the *recorded outcome* of a whole class of cases — `(/ 5 0)`,
`(% 5 0)`, `(/ Int64.min -1)`, a constant OOB index, `expect` on a constant `None` — from `(trap …)`
to `(error CDZ…)`. It is a real semantics decision, not an implementation detail, and it interacts with
the corpus's existing recorded traps. The two questions are **separate** and must not be conflated:
1. *May the fold fold a trapping op into a value?* — **No, never** (settled; the guard is right, and it
   is independent of everything below).
2. *May/should a SEPARATE pass reject a program proven to trap?* — the open decision.

**Two forces push rejection OUT of the initial Core/fold pass** (the operator's "defer to a later pass"
instinct is right):
- **Reachability.** `(if false (/ 5 0) 42)` never traps; a bottom-up fold sees the subexpression but has
  no reachability analysis, so rejecting on sight would reject correct programs. Sound rejection needs
  "unconditionally reached AND always traps" — a dataflow pass, not a rewrite.
- **The ragged boundary.** If rejection fires "wherever the analysis happens to be strong enough," then
  `(/ 5 0)` is a compile error but `(/ 5 (id 0))` compiles and traps — same bug, opposite outcome,
  decided by how much constant propagation ran. That unpredictability is why most languages do NOT
  reject arbitrary provable traps.

**Prior art (crisp-boundary designs).** Rust and Zig reject provable traps only in contexts where
compile-time evaluation is ALREADY mandatory — a `const`, a type-level value, an array length — where
the value is *required* to exist, so a trap producing it is a genuine "no such value" error. In ordinary
runtime position it stays a predictable runtime trap. This keeps the boundary principled and small.

**Options for the operator:**
- **(a) Reject a certain-trap only in compile-time-mandatory-eval contexts** (crisp, principled, small
  blast radius; matches Rust/Zig). Ordinary runtime position keeps the trap.
- **(b) A dedicated "certain-trap" diagnostic pass with reachability, uniform over ALL trap kinds**
  (div-by-zero, overflow, OOB index, `expect` on `None`). To avoid the ragged boundary, make it a
  **warning** in runtime position (surface the bug without a coverage-dependent hard gate), escalating
  to a hard error only in mandatory-eval position. Larger, but catches the whole class.
- **(c) Status quo** — traps stay runtime traps; the fold guard is the only obligation. Simplest;
  ships programs that trap.

**Status.** 🔴 **Operator decision.** No corpus/spec change made pending the call. If (a) or (b): a new
diagnostic code + corpus cases flipping the affected constant-trap cases from `(trap …)` to `(error …)`
in the mandatory-eval (and/or warning) contexts, and a `compiler-pipeline.md` requirement that a
Core→Core rewrite is meaning-preserving (a runtime trap stays a runtime trap; a rewrite may not
manufacture NOR erase a trap) — which formalizes what the fold guard already does and is worth landing
under ANY option. Related: the seed's known const-fold over-eager trap on `(% Int64.min -1)` (corpus
`06-numeric-model.sexp` line ~500, gated so it does not FAIL) is the mirror bug — the fold trapping a
case that should NOT — and the same "rewrites preserve traps exactly" requirement governs both
directions. Learning:
`spec/learnings/2026-07-06-constant-folding-must-preserve-runtime-traps.md`.

**Update (2026-07-06):** the meaning-preservation requirement now has a THIRD witness beyond partial
arithmetic — CONTROL flow. Folding a constant-condition `(if c t f)` must drop the untaken branch so a
trap/effect in it does not occur (`(if (< 1 2) 7 (% 5 0)) → 7`), the dual of the erase/manufacture
arithmetic faces. This strengthens the case for landing the `compiler-pipeline.md` "a Core→Core rewrite
is meaning-preserving" requirement independent of the certain-trap-rejection decision (a/b/c above): the
requirement demonstrably governs division folding, modulo-overflow folding, AND conditional folding, and
the same short-circuit-shielding reasoning governs `and`/`or` desugaring. Pinned by
`02-binding-and-control.sexp` "a conditional whose condition folds to a constant still drops the untaken
trapping branch" (PASS). Learning:
`spec/learnings/2026-07-06-folding-a-constant-condition-preserves-short-circuit-shielding.md`.

---
