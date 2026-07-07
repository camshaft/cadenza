# Constant folding must preserve runtime traps — and whether a certain trap should be a compile error is a separate decision

*2026-07-06*

**What happened.** The Cadenza-authored compiler's first Core→Core rewrite is constant folding:
a primitive over two constant operands collapses to a single `KConst`. Writing it surfaced that
folding division and modulo is not unconditional. `(/ c d)` and `(% c d)` **trap** at run time on a
zero divisor, and `/` additionally overflows (traps) on `Int64.min / -1`. Folding such a case to a
value would erase a trap the source specifies — a runtime trap must not silently become a
compile-time value. So the fold is guarded (`foldable-divisor`): it collapses a division/modulo to a
constant **only** when the divisor is a non-zero, non-overflowing constant; otherwise it rebuilds the
primitive around its folded operands, and the trap happens at run time exactly as written. This was
verified against the seed: `(/ 10 (- 3 3))` folds the divisor `(- 3 3)` to `0` but still emits a real
`i64.div_s` and **traps** (the trap is preserved, not erased); `(/ (+ 10 10) (+ 2 2))` folds fully to
`5` (it cannot trap); a bare `(/ 5 0)` compiles to a valid component that traps at run time (the seed
neither folds it to a value nor rejects it). The mirror bug already lives in the corpus: the seed's
own const-fold path over-eagerly reuses the division-overflow check on `(% Int64.min -1)` and wrongly
**traps** a case that must yield 0 (`06-numeric-model.sexp`, "modulo by -1 is zero even at the minimum
integer") — a fold *manufacturing* a trap, the dual of a fold *erasing* one.

Writing the guard raised a genuine policy question (flagged by the operator): **if the compiler can
prove a program will unconditionally trap, should it reject the program at compile time rather than
ship one that traps?** That instinct is safety-sound but it is a *different* question from what the
fold guard answers, and conflating them is the trap.

**Why.** Two questions must be kept separate:

1. *May the fold fold a trapping operation into a value?* **No, never** — an optimizer's contract is
   to preserve meaning, and the meaning of `(/ 5 0)` today is "trap" (a trap is a legitimate terminal
   condition, constitution §"a program terminates in exactly one terminal condition" — not an invalid
   program). Folding it to a value is a miscompile; folding it into a *rejection* also changes the
   meaning. The guard is the meaning-preserving floor and is correct independent of everything else.

2. *May/should a separate pass reject a program proven to trap?* This is the open decision, and two
   forces push it out of the initial fold pass — which is exactly why the operator's "defer to a later
   pass" instinct is right:
   - **Reachability.** `(if false (/ 5 0) 42)` never traps (verified: the seed returns 42). A
     bottom-up fold sees the `(/ 5 0)` subexpression but has no reachability analysis, so rejecting on
     sight would reject correct programs. Sound rejection needs "unconditionally reached AND always
     traps" — a dataflow pass, not a rewrite.
   - **The ragged boundary.** If rejection fires "wherever the analysis is strong enough," then
     `(/ 5 0)` is a compile error but `(/ 5 (id 0))` compiles and traps — same bug, opposite outcome,
     decided by how much constant propagation happened to run. That unpredictability is why most
     languages do not reject arbitrary provable traps; the ones that do (Rust, Zig) tie rejection to
     contexts where compile-time evaluation is *already mandatory* (a `const`, a type-level value, an
     array length), where the value is required to exist so a trap producing it is a genuine "no such
     value" error — keeping the boundary crisp and small.

The deeper lesson is that **"where an optimization runs is observable"** cuts both ways: the same
resolved-IR architecture that lets folding happen early (so the byte output has no dead code —
[[2026-07-06-the-front-rung-of-a-resolved-ir-compiler-needs-nested-payload-binders]]) also means the
fold is the first place a trap can be wrongly erased or manufactured. A Core→Core rewrite carries a
semantics-preservation obligation (optimizing-compiler catalog #6,
[[2026-07-06-optimizing-compiler-techniques-for-a-functional-immutable-ir]]): it may reduce operands,
but it may neither manufacture a trap the source did not denote (the `% Int64.min -1` bug) nor erase
one it did (folding `(/ 10 0)` to a value).

**The requirement it drove.** A conformance case in `06-numeric-model.sexp` —
*"a division whose divisor folds to zero still traps"* (`(/ 10 (- 3 3))` → `(trap "division by
zero")`) — pins the meaning-preservation floor from the *erase* direction, complementing the existing
"modulo by -1 is zero even at the minimum integer" case (the *manufacture* direction) and the guarded
`(if true (if true 5 (/ 1 0)) 9) → 5` reachability case in `02-binding-and-control.sexp`. Together
they witness that folding preserves traps exactly. The **open decision** — whether a provable-certain
trap becomes a compile-time rejection, and in which contexts — is recorded as item 9 in the compiler
spike's `implementation/SPEC-BACKLOG.md` for operator review, with three options (reject only in
compile-time-mandatory-eval contexts, à la Rust/Zig; a uniform certain-trap diagnostic pass that warns
in runtime position and errors in mandatory-eval position; or status quo). Under any option, a
`compiler-pipeline.md` requirement that a Core→Core rewrite is meaning-preserving — a runtime trap
stays a runtime trap, a rewrite manufacturing or erasing a trap is a miscompile — is worth landing,
since it formalizes what the fold guard already does and governs both the erase and manufacture bugs.
