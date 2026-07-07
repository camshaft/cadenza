# The workaround was the bug — correcting the "scale limit" diagnosis of the final self-host blocker

*2026-07-07*

**What happened.** Two cycles ago I documented the final self-hosting blocker (Tier 2f — `resolve` on a
runtime-built `Node` declining "runtime compound element of a kind the runtime cannot box yet") as a
**scale limit in the seed's runtime heap-boxer**: every tractable resolver compiled, only the full
18-variant `resolve` failed, and even `resolve` on a runtime `(NInt 42)` failed — so I concluded it was
a full-function union/scale property with no minimal witness, and (correctly, given that premise)
declined to pin a corpus case. That diagnosis was **wrong**, and the correction is the lesson.

The real cause was **self-inflicted**: `resolve`'s `PUnknown` arm was written as
`(Core.KConst (unknown-head-trap))` where `unknown-head-trap = (Bytes.len (Bytes.of (list 256)))` — an
**out-of-range `Bytes.of` used as a placeholder trap** for an unrecognized head (the interim stub I
flagged as SPEC-BACKLOG item 11). That one artificial arm poisoned the *whole* runtime-called `resolve`:
the out-of-range `Bytes.of` is a `Never`-typed value, and on the runtime-heap path the seed emitted an
invalid component for it, which is why *every* call to the function failed regardless of input.
Replacing the hack with a proper `Core.KError` variant that lowers to `unreachable` (an honest defined
trap, no Bytes) fixed it entirely — `bytes → component` then connected end-to-end. **The artificial
workaround WAS the bug.** And underneath it *was* a real seed invariant, just not the one I named: a
`Never`-typed value inside a runtime compound (or as a sum/tuple payload, or a call argument) emitted an
invalid component instead of a defined trap — a decline-don't-miscompile violation affecting any such
program, now hardened in the seed (a `Never` compound element short-circuits to `unreachable`; a
`Never`-bodied function stubs to `unreachable` keeping its non-Never signature; a `Never` call argument
diverges the call), pinned by a sibling's corpus case and memory
([[never-typed-value-on-the-runtime-heap-path]]).

**Why.** Two honest lessons, one methodological and sharper than the "scale limit" rule it corrects.
First: **my bisection reached a confident wrong conclusion because it varied the wrong axis.** I grew
resolvers arm-by-arm and saw them all pass, and inferred "only the full union fails → scale limit." But
the failing arm — the `Bytes.of (list 256)` hack — was *not in my reconstructions*; I rebuilt the
*structural* shape (the variant count, the tuple arities) and omitted the one *content* detail (the
out-of-range Bytes) that actually caused it. A bisection that reduces the wrong dimension confirms a
false hypothesis: I proved "structure isn't the cause" and misread it as "scale is the cause," when the
cause was a specific poisoning value I had abstracted away. The corrected rule: **when every reduction
of a failure passes, suspect that the reduction is dropping the culprit, not that the culprit is
emergent scale** — reduce by *deleting arms of the actual failing program* (which would have isolated
the `PUnknown` arm immediately), not by *rebuilding a clean analogue*. Second, and this is the
vindication the spike itself calls out: **"write it honestly, don't contort around gaps" is not just
style — the contortion can be the defect.** The Bytes-hack trap was a workaround for the absence of a
diagnostics channel (backlog item 11); writing the honest thing (`KError → unreachable`) both removed
the bug and was the correct design. A placeholder that reaches for an unrelated mechanism (an
out-of-range Bytes to force a trap) can poison analysis and emission in ways the honest form never
would.

**The requirement it drove.** No new corpus case from me — the sibling pinned the real invariant (*"a
recursive resolver whose trapping arm builds a compound compiles"* in `05-compound-types.sexp`, which
passes), and my resolver-join case from last cycle still passes, so the capability is doubly witnessed.
The durable output is this correction: **SPEC-BACKLOG item 16 is withdrawn as mis-framed** (there was no
seed scale limit; the "cannot box" decline was a self-inflicted Bytes hack over a real but
differently-shaped `Never`-on-heap invariant, now fixed), and **item 11 (the unknown-head placeholder
trap) is resolved** — the honest `KError → unreachable` variant is exactly the real diagnostic-marker
the item asked for (a defined trap, not an out-of-range-Bytes hack; a proper `CDZ` code can still layer
on later when the diagnostics channel lands, but the miscompiling placeholder is gone). The prior two
learnings that called 2f a "scale limit" are left in place as the historical record, corrected here
rather than rewritten — the wrong diagnosis and its correction are both part of the honest trail, and
the meta-lesson (reduce the failing program, not a clean analogue) only exists because the first
diagnosis was made and then overturned.
