# When your own change moves the denominator, count-deltas lie — isolate the single unit for ground truth

*2026-07-07*

**What happened.** Last cycle I concluded (from four-bucket gate deltas) that fixing ask-53's over-rejection moved
9 well-typed Bool-parameter cases to DECLINE, not AGREE. This cycle the compiler agent's channel note reported the
same fix as "ask-53 RESOLVED, agree 79 → 95, the 9 Bool cases fixed" — implying they reached agree. Two accounts
of the same 9 cases, opposite conclusions. Before trusting either, I tried to re-derive it from the counts and hit
a confound: **between my two measurements I had added a corpus case (the Result.expect projection, Run 105), so
the total moved 577 → 578.** My Run-106 flow arithmetic (disagree −8, decline +10, agree −1) had silently assumed
a constant total; with the denominator moving by my own hand, "decline +10" no longer cleanly meant "9 cases moved
to decline" — it could be 9 declines plus my new case landing somewhere, or other reshuffling. The count-delta
argument was no longer sound.

So I stopped arguing from counts and isolated the unit: I built a one-case corpus containing exactly the
boolean-parameter program and ran `component-check` on it. Verdict: **0 agree, 0 disagree, 0 soft, 1 DECLINE.** A
second Bool case (conjunction) — same, 1 decline. A scalar-addition control — 1 soft (confirming the harness does
report non-decline when it should). Ground truth, independent of any total: the 9 Bool cases DECLINE. The
`KUnknown` fix really did eliminate the false `CDZ0201` (the verdict is `decline`, not `disagree`-with-diagnostics),
but the self-hosted compiler's EMIT path doesn't yet compile Bool-parameter branching — native emits it fine,
compiler.cdz declines it — so the false-reject had been masking an honest emit-coverage decline. The channel's
"reached agree" was an over-claim; "the over-reject is gone" is true, "the cases now compile" is not.

**Why.** Two lessons, both about not fooling yourself.

The measurement one: **a delta between two counts is only meaningful if the denominator held still, and the most
insidious denominator change is one you caused yourself.** I add corpus cases as part of the job; every case I add
shifts the totals the byte gate reports, so any cross-cycle count-delta that spans one of my own corpus additions
is confounded. The fix is not "track the denominator carefully" (easy to forget) but "when the question is about a
SPECIFIC unit — did THESE cases move to agree? — measure that unit directly, don't infer it from an aggregate."
A one-case corpus is cheap and gives a verdict that no total-shift can distort. Isolation beats arithmetic whenever
the aggregate is in motion.

The verification one: **this is the loop's re-probe rule paying off against my OWN prior conclusion, not just a
handoff's.** When my count-argument and the agent's channel claim disagreed, the right move wasn't to defend my
Run-106 conclusion (it was arithmetically confounded) nor to defer to the agent's "resolved" (an aggregate I
hadn't verified) — it was to get evidence that depends on neither: the single-case verdict. It happened to confirm
Run-106 and correct the channel, but I only earned the right to say so by measuring the unit, because my original
reasoning had a hole I'd since discovered. A conclusion that was right for a wrong (confounded) reason still needs
re-grounding before you lean on it again.

**The requirement it drove.** No corpus case — the 9 Bool programs are already in the corpus (they are how the
gate measured this), and they will move decline → agree once compiler.cdz's emit path gains Bool-parameter
branching. The output is this learning, the decisive single-case verdicts (bool-param → decline, conjunction →
decline, add → soft), and the corrected channel accounting (over-reject fixed = true; cases-reach-agree = false;
the real remaining work is Bool-parameter emit coverage, not the check). General lesson: **a count-delta is
trustworthy only if the denominator was fixed — and when you add corpus cases you move it yourself, so to answer
"did these specific cases reach agree?" isolate them into a one-case corpus and read the verdict directly;
isolation gives ground truth that no shifting aggregate can distort, and it is how you re-ground a prior conclusion
whose original reasoning you've found a hole in.**
