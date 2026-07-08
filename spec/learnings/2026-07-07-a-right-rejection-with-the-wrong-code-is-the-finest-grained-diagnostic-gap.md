# A right rejection with the wrong code is the finest-grained diagnostic gap — the last rung before agree is category resolution, and it's a lattice question

*2026-07-07*

**What happened.** ask-55's shape-check fix (float node → decline, not crash) did more than stop the crash: the
int/float mix cases (`(+ 1 4.5)`, `(- 1 4.5)`, `(< 1 4.5)`, bitwise/shift/ordering with a float operand) now
REJECT — they emit a diagnostic — where a few cycles ago they were under-accepted, then crashed, then declined.
So all 14 remaining byte-gate disagreements collapsed to a single class: `native=CDZ0301,
comp=diagnostics[CDZ0201]`. The rejection is correct; only its CODE is wrong. Native distinguishes CDZ0301
("numeric types do not silently promote" — both operands numeric, different kinds: int vs float) from CDZ0201
("operation on mismatched types" — a non-numeric mismatch: int vs bool). compiler.cdz collapses both to CDZ0201,
because its kind lattice has no float-numeric kind to tell "both numeric, one is float" from "non-numeric
mismatch."

**Why.** Watching one family of cases travel the full outcome ladder in a handful of cycles makes the ladder's
structure legible. The int/float-mix cases went: **under-reject** (compile a program native rejects — wrong, a
missing rejection) → **crash** (ask-55's regression — worse, a trap on the way to rejecting) → **decline/reject**
(honest: the compiler refuses) → **right-rejection-wrong-code** (rejects, but miscategorizes why) → and the last
step, not yet taken, is **agree** (rejects with the code native uses). Each rung is strictly better than the last
on the reject-don't-miscompile ordering, and the top of the ladder has a rung finer than "does it reject?": *does
it reject for the RIGHT STATED REASON?* A diagnostic is not just a boolean rejection; it is a rejection with a
category, and the category is part of the contract (a consumer branches on the code; `diagnostics.md` makes the
code a stable requirement). So "rejects with the wrong code" is a real disagreement — the finest-grained one — and
the corpus is right to score it disagree, not agree.

The load-bearing point about the FIX: a wrong-code diagnostic is a **lattice-resolution** problem, not a logic
bug. compiler.cdz emits CDZ0201 for the int/float case not because its rejection logic is wrong but because its
kind lattice is too COARSE to draw the distinction the code encodes. CDZ0301 exists precisely to say "these are
both numbers, I just won't promote between them" — a statement that requires recognizing "float is numeric but not
i64." A two-or-three-value lattice (`Ki64`/`KBool`/`CKUnk`) can say "these kinds differ" (→ CDZ0201) but cannot
say "these differ AND both are numeric" (→ CDZ0301). So the diagnostic code the compiler can emit is bounded by
the resolution of the kind lattice it reasons over: **to emit a more specific diagnostic, the compiler needs a
more specific type kind, because the code IS a claim about kinds.** This is the same lattice-enrichment thread as
ask-53 (`KCompound` to reject compound-in-scalar-position, `KUnknown`/`CKUnk` to stay silent on the unprovable) —
each finer diagnostic distinction the compiler wants to draw requires the lattice to carry the distinction the
diagnostic asserts. Coarsen the lattice and diagnostics collapse into their nearest general code; enrich it and
they split into the specific ones.

**The requirement it drove.** No new corpus case — the 14 CDZ0301 cases are already pinned (that is how the gate
scored the code mismatch), and they move disagree → agree when compiler.cdz gains a float/numeric kind and emits
CDZ0301 for the both-numeric case. The output is ask-56 (the wrong-code gap: emit CDZ0301 not CDZ0201 when both
operands are numeric but differ, keep CDZ0201 for numeric-vs-non-numeric — with the discriminator that both codes
must be tested) and this learning; ask-55 moved to pending-validation (the crash is fixed, floats decline/reject).
WRONG=0 holds (a wrong-code rejection is not a wrong value). General lesson: **the outcome ladder has a rung finer
than "does it reject?" — "does it reject with the right CODE?" — and a wrong-code diagnostic is the finest-grained
disagreement; the fix is lattice resolution, not logic, because a diagnostic code is a claim about kinds, so a
compiler can only emit a distinction its type lattice can draw — enrich the lattice and the codes split, coarsen
it and they collapse into the nearest general one.**
