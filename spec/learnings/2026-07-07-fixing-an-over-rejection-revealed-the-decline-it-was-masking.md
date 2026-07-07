# Fixing an over-rejection revealed the decline it was masking — a wrong classification can sit on top of a truer, less-flattering one

*2026-07-07*

**What happened.** The `KUnknown` half of ask-53 landed: the self-hosted compiler's `check-node` pass no longer
emits `CDZ0201` for an operand whose kind it cannot positively prove (a Bool that arrives as a function parameter,
a call result, or a match scrutinee). Last cycle this was the sharp finding — 9 well-typed Bool-parameter programs
were being FALSE-REJECTED (`native=ok`, compiler `diagnostics[CDZ0201]`). After the fix, the byte gate moved:
disagree 102 → 94, the 9 Bool over-rejects → 0, and no new wrong-values (WRONG=0 holds, 0 `native=trap`, 0 true
miscompiles). The over-rejection — the more urgent half, because it rejected GOOD programs — is gone.

But reading the vocabulary deltas precisely told a subtler story. The 9 cases did NOT become `agree`. Agree went
96 → 95 (−1), soft stayed 25, and **decline went 354 → 364 (+10)**. So the 9 well-typed Bool programs moved from
DISAGREE (false-reject) to DECLINE — not to correct compilation. The false `CDZ0201` had been sitting ON TOP OF an
underlying decline: the compiler does not yet fully compile Bool-parameter branching, so its true state for these
programs was always "I can't handle this" (decline). The over-rejection masked that — a program that should have
read as an honest decline was instead reading as a confident (wrong) rejection. Removing the false diagnostic
didn't compile the program; it uncovered the decline that was the real state all along.

**Why.** This is a specific, recurring shape worth naming: **a wrong classification can be layered over a truer
one, and fixing the wrong layer reveals the layer beneath — which is often less flattering than the fix made it
look.** The natural narrative after a fix is "9 false-rejects gone → 9 wins," but the honest accounting is "9
false-rejects became 9 honest declines" — a real improvement on the reject-don't-miscompile ordering (decline >
false-reject, because a decline is honest about the compiler's limits while a false-reject slanders correct code),
but NOT the payoff the disagree-count drop superficially suggests. The case only truly lands (reaches `agree`)
when the compiler can POSITIVELY handle it — for a Bool parameter, that means propagating the parameter's declared
Bool kind through to the branch check, not merely declining to judge it. The `KUnknown` fix did the right thing
(stop lying about correct code) but it is one of two steps: stop the false-reject (done), then supply the missing
positive capability (the parameter-kind propagation) that turns the decline into an agree. Conflating "the
false-reject is gone" with "the case works" would have over-claimed the win by exactly the gap between decline and
agree.

The measurement lesson that made this visible: **the disagree count alone is a lossy summary; the win is only real
if the cases moved to `agree`, and you must check WHERE they went, not just that they LEFT disagree.** The
ask-33 runtime-behavior classifier gives four buckets (agree/soft/disagree/decline) precisely so this is legible
— disagree dropping by 8 while decline rose by 10 and agree fell by 1 is a completely different event from
disagree dropping by 8 into agree. A loop that reads only "disagree went down" would have recorded a payoff that
didn't happen. The four-bucket delta, read as a flow between buckets rather than a single headline, is what
distinguishes "fixed" from "reclassified."

**The requirement it drove.** No new corpus case — the 9 Bool programs are already in the corpus (they are how the
gate measured the over-rejection, and they will move decline → agree when the parameter-kind propagation lands, no
new case needed). The output is this learning and the corrected accounting on the ask-53 progress: the `KUnknown`
over-reject half is FIXED (9 false-rejects eliminated, WRONG=0), but the cases moved to decline, so the remaining
work is positive Bool-parameter kind propagation (to reach agree) plus the still-open `KCompound` under-reject
half (the 89 `native=rejected` disagrees, ask-30's missing type-checker). General lesson: **a wrong classification
can mask a truer, less-flattering one; when a fix drops the disagree count, read the four-bucket FLOW to see where
the cases went — "left disagree" is not "reached agree," and a false-reject becoming an honest decline is real
progress but not the payoff, because the case only lands when the compiler gains the positive capability to
compile it.**
