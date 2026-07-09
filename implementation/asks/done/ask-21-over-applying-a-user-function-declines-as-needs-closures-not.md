## 21. ⚪ Over-applying a user function declines as "needs closures", not the CDZ0201 the corpus says it mirrors — and head-position name classification is fragile

**Finding.** `(f 5 9)` on a unary `f` declines *"call to f with 2 args, expected 1 (partial application
needs closures)"*. But the corpus records the parallel constructor over-application `(Some 1 2)` as
`(error CDZ0201)` (apply-a-non-function), and `09-functions.sexp`'s prose says a user-function
over-application is "arity-checked the same way" — yet only the constructor case is pinned, and the
seed treats the user-function case as a closure-feature gap rather than the type error it is (`(f 5 9)`
= `((f 5) 9)`, applying the Int64 `6` to `9`).

**Why it touches the seed (not the spec).** The recorded semantics already imply CDZ0201 (the
single-arity desugaring is the same as the constructor case); the seed's divergent "needs closures"
decline is the gap. **A second, deeper signal:** pinning this as a corpus case FAILed the gate via a
cross-case interaction — adding `(f 5 9)` flipped an unrelated passing case (`(let ((ctor None)) (ctor
unit))`, binding+applying the prelude constructor) to a wrong *"CDZ0401: undeclared capability: ctor"*.
So head-position name classification (is a head a bound value / a constructor / a capability / an
over-applied function?) is order-sensitive and destabilizes when a new over-application case is added.

**Status.** ⚪ Seed work. **No corpus case** — pinning `(f 5 9) → CDZ0201` broke the gate (the cross-case
`ctor`-misread-as-capability regression), which the corpus discipline forbids; pin it once the seed
classifies user-function over-application as `CDZ0201` and head-position classification is total and
order-independent. Scope: not just "emit CDZ0201 for over-application" but "make head-position name
classification total across value / constructor / capability / over-applied-function." The trigger was
a transient mid-refactor `compiler.cdz` (a `kind-of` call/def arity mismatch), not a compiler
regression. Learning:
`spec/learnings/2026-07-07-over-applying-a-user-function-declines-as-closures-not-as-an-arity-error.md`.

---

**🔎 LOOP-PROBED 2026-07-07 (stable seed 17:00, SHA OK) — ROOT CAUSE pinpointed to ONE branch, and the
head-classification fragility is now GONE. CONFIDENCE: HIGH (source-located + reproduced).**

Reproduced and localized. The gap is a **reject/decline ASYMMETRY between two arity-check sites** that should
agree, plus a single branch that conflates over- with under-application:

- **Constructor over-application ALREADY rejects correctly:** `(Some 1 2)` → `reject("CDZ0201",
  "over-applying a single-arity constructor")` at **`codegen.rs:3234`**. ✓ (matches corpus `09-functions.sexp:180`
  `(error CDZ0201)`.)
- **User-function arity mismatch DECLINES for BOTH directions** at **`codegen.rs:7136-7141`**:
  ```rust
  if args.len() != f.params.len() {
      return decline(format!(
          "call to `{name}` with {} args, expected {} (partial application needs closures)", …));
  }
  ```
  This single `!=` branch handles over- (`args > params`) and under- (`args < params`) application IDENTICALLY —
  but they are DIFFERENT: over-application `(f 5 9)` desugars to `((f 5) 9)` = **apply-a-non-function** (the
  Int64 result of `(f 5)` applied to `9`), the exact CDZ0201 class the seed already rejects for `(5 3)`
  ("applying a non-function", codegen.rs) and `(Some 1 2)`; whereas under-application `(f)` is a genuine
  partial-application/closures FEATURE gap that correctly declines.

**⚠️ The CLI display masks that this is a code asymmetry, not a message one.** `main.rs:185/212` print
`"declined: {}"` using ONLY `d.0` (the message) — never `d.code()`. So `(Some 1 2)` (internally
`reject CDZ0201`) and `(f 5 9)` (internally a codeless `decline`) BOTH print `declined: …` at the CLI, hiding
that one carries CDZ0201 and the other carries no code. Confirm the asymmetry by the SOURCE sites (3234 vs 7137),
not the CLI text. It is the byte-level `component-check` that sees the difference: native emits CDZ0201 for the
user-fn over-application, the seed emits a codeless decline → scored `disagree` (the constructor case scores
`agree` because both sides carry CDZ0201).

**Behavior-gate status (why this isn't a FAIL today):** all four over-application corpus cases PASS
(`over-applying a constructor is a type error`, `…by several arguments`, `applying a non-function is a type
error`, `a nullary variant applied to a non-unit payload`) — the behavior gate accepts a decline where the
corpus records CDZ0201 via the doc's "**or declines if it does not yet check it**" reject-don't-miscompile
carve-out. Gate 572/0. So this gap lives ONLY at the byte-level `component-check` (coded-rejection mismatch) and
in classification precision — NOT the behavior gate.

**Proposed fix (CONFIDENCE: HIGH):** split the `codegen.rs:7136` branch by direction:
```rust
if args.len() > f.params.len() {
    return reject("CDZ0201", format!("over-applying `{name}`: {} args, expected {} \
        (the result of the saturated call is applied to a non-function)", args.len(), f.params.len()));
}
if args.len() < f.params.len() {
    return decline(format!("call to `{name}` with {} args, expected {} \
        (partial application needs closures)", args.len(), f.params.len()));
}
```
This makes user-fn over-application match the constructor case (3234) and the `(5 3)` non-function case — the
three "apply-a-non-function" sites then all `reject CDZ0201`, and under-application keeps its honest closures
decline. That moves the user-fn over-application `component-check` cases `disagree → agree` without touching the
behavior gate (still a rejection; the carve-out still holds).

**Head-classification fragility (the ask's "second, deeper signal") — NOW RESOLVED on stable 17:00.** Re-probed
the exact regression case: `(let ((ctor None)) (ctor unit))` → **VALID** (was the `CDZ0401: undeclared
capability: ctor` cross-case misread). `(let ((s Some)) (s 5))` → VALID too. So a let-bound constructor applied
in head position is now classified correctly; the order-sensitivity the ask flagged is no longer reproducible.
That removes the blocker the ask named for pinning a corpus case — **`(f 5 9) → CDZ0201` can be pinned once the
7136 split lands** (the head-classification regression that previously broke gate-pinning is gone).

**Acceptance signal (updated):** after the 7136 split, `(f 5 9)`/`(g 1 2 3)` → `reject CDZ0201` (CLI still prints
`declined:` but `d.code()==CDZ0201`); under-application `(f)` stays a decline; `(let ((ctor None)) (ctor unit))`
stays VALID (no regression); then pin `(f 5 9) → (error CDZ0201)` as the corpus companion to the existing
`(Some 1 2)` case, and it holds without the cross-case break.

---

## ✅ DONE 2026-07-07 (conformance loop) — over-application rejects CDZ0201; under-application stays a decline

**Fixed** at the pinpointed site (`codegen.rs` ~7136). Split the single `args.len() != f.params.len()` decline
into two: `args > params` (over-application — a type error, `((f 5) 9)` applies a non-function) ⇒
`reject("CDZ0201", "over-applying `f`: …")`, matching the constructor over-application `(Some 1 2)` at
`codegen.rs:3234`; `args < params` (under-application — a well-typed partial application the seed can't lower)
⇒ the existing `decline` (needs closures).

**Head-classification fragility did NOT recur:** the change is scoped to the branch AFTER `f` resolved to a
user function (`lookup_fn` succeeded), not the head-resolution path that once misread a bound `ctor` as a
capability. Verified: `(f 5 9)` → `Rejected(CDZ0201)`, and the prior regression case
`(let ((ctor None)) (ctor unit))` → `Value("(None unit)")` (well-typed, no CDZ0401). Both pinned as `probe`
regressions in `ill_typed_programs_are_rejected_not_crashed`.

**Gates:** BEHAVIOR 572/0, IGNITION byte-identical, COMPONENT-CHECK 577 agree/0 disagree, cargo test green.
📦 STABLE refreshed. Learning: `user-function-over-application-rejects`. NO corpus case (ask-21 notes pinning
`(f 5 9)` in the corpus previously broke the gate via the cross-case regression; the compiler-mechanics
assertion lives in the `probe` suite). Remaining broader scope ("make head-position classification TOTAL across
value/constructor/capability/over-applied-fn") is untouched — this closes the concrete over-vs-under asymmetry.
