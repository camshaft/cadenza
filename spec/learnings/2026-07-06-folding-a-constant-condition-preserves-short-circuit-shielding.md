# Folding a constant-condition conditional must preserve short-circuit shielding — the third face of trap-preserving rewrites

*2026-07-06*

**What happened.** The compiler-in-Cadenza spike grew its constant-fold pass past arithmetic and
comparisons to conditionals: `fold-if` reduces `(if c t f)` when its condition `c` folds to a constant
Bool by *becoming the taken branch and dropping the other*. The dropped branch is never lowered, so a
trap or effect in it never occurs — which is correct precisely because a run-time conditional already
shields its unselected branch (core-semantics.md §Conditionals Evaluate One Branch). Verified against
the seed: `(if (< 1 2) 7 (% 5 0)) → 7` — the condition `(< 1 2)` folds to true, and the untaken
`(% 5 0)` (a modulo-by-zero that would trap) is dropped, not evaluated. The subtlety is that the
shielding must hold *after* the condition itself folds: a fold that eagerly evaluated both branches, or
that kept the trapping branch "to be safe," would manufacture a trap the source shields.

**Why.** This is the third face of one principle already recorded for division
([[2026-07-06-constant-folding-must-preserve-runtime-traps]]): **a Core→Core rewrite must preserve a
program's trap behavior exactly — it may neither erase a trap the source denotes, nor manufacture one
the source does not.** The three faces the spike has now surfaced:
- **Don't erase** — `(/ 10 (- 3 3))` folds the divisor to 0 but must still trap; folding it to a value
  erases a trap. (Guarded by `foldable-divisor`.)
- **Don't manufacture (arithmetic)** — the seed's over-eager const-fold wrongly traps `(% Int64.min -1)`
  which must yield 0; applying the division-overflow check to modulo manufactures a trap.
- **Don't manufacture (control)** — `(if (< 1 2) 7 (% 5 0))` must drop the untaken trapping branch;
  keeping or evaluating it manufactures a trap the conditional shields.

The control face is the one a naive folder gets wrong most easily, because folding a conditional is
usually framed as an *optimization* ("the condition is known, inline the branch") rather than as a
*semantics-preservation obligation* ("the untaken branch must remain unevaluated"). Framing it as the
latter makes the rule fall out: `fold-if` folds the condition, and *only if* it became constant does it
select a branch and discard the other — the discard is not an optimization to justify, it is the
run-time shielding semantics carried into compile time. This is also why `fold-if` recurses into the
branches *before* checking the condition's constancy is unnecessary for the taken branch but harmless,
and why it must never lift a branch's trap out to the fold: the branch's evaluation is conditional, and
the fold must keep it conditional (or drop it), never make it unconditional. The same reasoning will
govern the boolean connectives (`and`/`or` desugar to `if`, so their short-circuit shielding is the
same property) and any future rewrite that chooses among sub-expressions.

**The requirement it drove.** A conformance case in `02-binding-and-control.sexp` — *"a conditional
whose condition folds to a constant still drops the untaken trapping branch"*
(`(if (< 1 2) 7 (% 5 0)) → 7`) — pins the control face. It is deliberately distinct from the existing
literal-`true` shielding case (*"a conditional shields a trap in a nested unselected branch"*): there
the condition is already a literal, so no fold is involved; here the shielding holds only *after* the
comparison condition folds, so it exercises the fold's short-circuit-preservation specifically. The
case PASSES today (the seed folds and shields correctly), turning the property into a permanent gate
obligation. It joins the divisor-folds-to-zero case (the erase face) and the modulo-at-minimum case
(the arithmetic manufacture face) as the third witness that folding preserves trap behavior in every
direction, and it strengthens the standing recommendation (SPEC-BACKLOG item 9) that
`compiler-pipeline.md` gain an explicit requirement that a Core→Core rewrite is meaning-preserving —
now demonstrably covering control flow, not only partial arithmetic.
