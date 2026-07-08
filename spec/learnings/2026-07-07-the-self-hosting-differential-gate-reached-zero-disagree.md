# The self-hosting differential gate reached zero disagree — the residual is honest declines and soft byte-differences, not miscompiles

*2026-07-07*

**What happened.** The self-hosted compiler component (`compiler.cdz` compiled by the reference seed, run over the
whole corpus via `component-check`) reached **0 disagreements**: 120 agree / 0 disagree / 25 soft / 434 decline.
`COMPONENT-CHECK: PASS`. Every program in the corpus that the self-hosted compiler handles now either produces
native's value (agree), produces different bytes that run to the same value (soft), or is honestly declined
(unimplemented feature, refused with a stub) — and NONE produces a wrong value, a wrong-coded diagnostic, or a
crash. The final cluster to fall was ask-56 (int/float mix rejecting with CDZ0201 where native emits CDZ0301);
adding a float/numeric kind to the check's lattice let it emit the right code, and the discriminator holds both
ways (`(+ 1 4.5)` → CDZ0301 agree, `(+ 1 true)` → CDZ0201 agree).

**Why.** This is worth recording as a milestone, but the durable content is HOW the last stretch was closed,
because it validates the whole measurement discipline the loop was built on. The ask-30 type-rejection frontier
did not fall as "implement the type checker." It fell as a sequence of distinct outcome-quality transitions, each
one a rung on the reject-don't-miscompile ladder (wrong-value < crash < decline < wrong-code < correct), and the
byte gate's four-bucket classifier (agree/soft/disagree/decline, ask-33) was what made each rung visible and kept
the loop from mistaking a lateral move for progress:

- **under-reject → decline** (the conservative check, ask-53): stop accepting ill-typed programs, but honestly
  decline the ones the coarse lattice can't judge rather than false-reject them.
- **decline → crash → decline** (ask-55): a lattice extension trapped on an unmodeled node kind (float); the fix
  was "unrecognized kind → silent decline, never trap."
- **decline/under-reject → reject-with-a-code** (coded diagnostics, the `KError`-payload-carries-the-code work):
  the ones provable from the type (bool exhaustiveness) reached agree by emitting native's code.
- **reject-wrong-code → reject-right-code** (ask-56): the finest rung — a diagnostic code is a claim about kinds,
  so emitting CDZ0301-not-CDZ0201 required the lattice to distinguish "both numeric, different kind" from
  "non-numeric mismatch."

Each of those was a real, separately-measured, separately-verified step, and several times the raw disagree count
moved the RIGHT way while the underlying state got WORSE or merely lateral (the 9 Bool cases that went to decline
not agree; the float regression that dropped disagree 85→22 while introducing 22 crashes). The loop caught those
by reading the four-bucket flow and isolating single cases, not the headline. So "0 disagree" is trustworthy
precisely because every step to it was audited for severity, not just count — the milestone is only meaningful
because the measurement that produced it distinguished "left the disagree bucket" from "reached agree" at every
stage.

What 0 disagree does NOT mean, and the loop must keep stating plainly: it is NOT "the compiler is complete." 434
declines remain — every one an unimplemented feature (runtime compounds as results, float equality, closures, user
sums, effects at scale) that the compiler honestly refuses rather than miscompiles. The differential gate proves a
NEGATIVE — "the self-hosted compiler never disagrees with native on what it does handle" — which is exactly the
reject-don't-miscompile contract: correctness is never traded for coverage. Coverage (turning declines into
agrees) is the ongoing work; soundness (never a wrong answer) is what 0 disagree certifies. The two are different
axes, and the gate's honesty is that it shows both — a rising agree count AND a large decline count AND zero
disagree, rather than collapsing them into one "percent passing" that would hide which is which.

**The requirement it drove.** No new corpus case — the 0-disagree state is measured over the existing corpus, and
the cases that closed the frontier (the CDZ0301 cluster) were already pinned; they moved disagree → agree as
ask-56 landed. The output is this milestone learning and the confirmed state (120 agree / 0 disagree / 25 soft /
434 decline, native gate 574/0, 0 traps, discriminator verified both ways). General lesson: **a self-hosting
differential gate reaching zero disagree certifies SOUNDNESS (never a wrong answer on what it handles), not
COMPLETENESS (the large decline count is the remaining coverage) — and the milestone is trustworthy only because
every step to it was audited on the reject-don't-miscompile severity ladder via the four-bucket flow, never by the
headline disagree count, which repeatedly moved the right way while the underlying state was lateral or worse.**
