# A const-fold placeholder must be unobservable — decline every use that would read it, so a dead slot can never leak as a wrong value

*2026-07-07*

**What happened.** The const-compound-folding tier advanced again (agree 126 → 129): let-bound compound
projections now fold, `(let ((p (record (x 1) (y 2)))) (. p y))` → 2, `(let ((t (tuple 5 9))) (tuple.0 t))` → 5,
and they compose. The mechanism is constant propagation through a new literal-compound environment (`lce`): when
`read-let` binds a name to a literal compound, it records the binding in `lce` (mapping the slot to the
compound's byte-offset) and binds the name's actual runtime slot to a PLACEHOLDER `(NInt 0)` — because the slot
numbering must stay consistent even though the compound value is never materialized. A projection off that name
resolves through `lce` to the compound's offset and folds to the scalar element; the placeholder slot is dead.

The hazard is the placeholder. A dead slot holding `(NInt 0)` is a loaded gun: if any code path ever reads that
slot as a VALUE — a bare use `(let ((t (tuple 5 9))) t)`, an equality `(= t …)` — the compiler would emit `0`
where native yields the actual tuple, which is a silent WRONG-VALUE miscompile, the worst outcome on the
reject-don't-miscompile ordering. The design defends against exactly this: `read-node`'s local-variable path
checks `lce` and DECLINES a compound slot. So the only use that succeeds is the projection (which never touches
the placeholder — it reads through `lce` to the real offset); every other use of a compound-let binding declines.
I verified: bare use → decline (not agree-with-0, not disagree), projection → agree (folds to the right scalar),
and the full byte gate → 0 disagree. The placeholder is structurally unobservable.

**Why.** This is the reject-don't-miscompile ordering applied to an OPTIMIZATION's blind spot, and it names a
discipline for any transform that leaves a stand-in in the program. A const-fold that replaces a value with a
placeholder has split the uses of that binding into two classes: the ones the fold HANDLES (here, projection —
resolved through the side table) and the ones it DOESN'T (bare use, equality, anything that reads the slot
directly). The handled uses are correct; the unhandled uses would read the placeholder and miscompile. The safe
design is not "make the placeholder a plausible value" (there is no value that is correct for an un-materialized
compound) but "**make every unhandled use DECLINE**" — convert the optimization's incompleteness into an honest
decline rather than a wrong answer. The placeholder is then never observable as a value, because the only paths
that reach it are dead (folded away) or declined. This is the general shape: **an optimization that introduces a
stand-in must guarantee the stand-in is unobservable, and the way to guarantee it is to decline — not
best-effort-emit — every use the optimization doesn't fully handle.** A stand-in that can leak is worse than not
optimizing at all, because not optimizing declines honestly while a leaked stand-in miscompiles silently.

And the loop's role: this is precisely the class of bug the differential byte gate exists to catch, and the check
that MATTERS is the negative one. It would be easy to test only the projection cases (they agree — the feature
works) and miss that a bare use leaks the placeholder. The verification that earns confidence is the adversarial
one — feed the binding to the uses the optimization does NOT handle and confirm they DECLINE, not that they
happen to return something. 0 disagree over the whole corpus is the certificate that no placeholder leaks, but the
targeted bare-use probe is what makes the safety legible rather than incidental.

**The requirement it drove.** No new corpus case — the bare-compound-let-binding case and the projection cases are
already pinned (the bare use as a decline, the projections now as agrees), which is exactly how the byte gate
certifies the placeholder is unobservable (bare use declines, 0 disagree). The output is this learning and the
verified safety invariant (placeholder never leaks: bare use → decline, projection → agree, full sweep → 0
disagree). General lesson: **an optimization that leaves a placeholder/stand-in in a dead slot must make it
unobservable by DECLINING every use it doesn't fully handle — never emitting a best-effort value, because there is
no correct value for an un-materialized thing and a leaked stand-in is a silent wrong-value miscompile; and the
verification that earns confidence is the adversarial negative one (confirm the unhandled uses decline), not the
positive one (the handled uses agree).**
