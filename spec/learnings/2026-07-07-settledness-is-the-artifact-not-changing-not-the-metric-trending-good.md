# Settledness is the artifact ceasing to change, not the metric trending toward good — a converging count on a live file is still noise

*2026-07-07*

**What happened.** Probing mid-cycle, I caught the self-hosted compiler in the middle of the runtime-compound
(M2) emission work — the byte gate showed **70 disagree, all traps** (`run error`), on compound-heavy cases
(tuple/record/list/map equality, member access, nested compounds). Re-emitting a minute later: **33 disagree, all
traps**. The count was falling — 70 → 33 — which is tempting to read as "a regression being progressively fixed,
converging toward clean." It was not that. It was `compiler.cdz` being actively rewritten (mtime advancing every
few seconds, size climbing 195k → 197k → 198k), and each emit sampled a different half-wired intermediate state.
When I stopped trusting the falling count and instead POLLED THE FILE'S MTIME until it held stable for three
consecutive reads (~60s unchanged), the settled emit was **0 disagree, 0 traps, agree 134 → 136** — coverage up,
fully sound. The 70 and the 33 were both pure work-in-progress noise; neither was a real state.

**Why.** I have written before that "a timeout/breakage is triage not verdict — re-run before recording"
(Run 82, Run 102, Run 119). This cycle sharpens the rule with the specific trap I nearly fell into: **when the
metric is CONVERGING (70 → 33 → toward 0), it looks like signal — like a regression being fixed in real time — and
that appearance is exactly what makes a mid-edit read dangerous.** A monotonically-improving count invites a
narrative ("they broke it, now they're repairing it, I'm watching the recovery"), and that narrative feels more
authoritative than a random-looking number, so it is MORE likely to be recorded as a finding. But a converging
count on a live file is not a recovery trajectory; it is a sequence of unrelated snapshots of different
intermediate states, and the "convergence" is an artifact of sampling a build that happens to be getting more
complete. The magnitude and the trend of the metric carry NO information about settledness. The only signal that
tells you a measurement is real is that **the thing you are measuring has stopped changing** — the artifact's
mtime/size holding constant across several polls, not the metric looking better.

So the discriminator for "may I record this?" is not "is the number good / trending good / stable-looking?" — it
is "has the ARTIFACT settled?", answered by watching the artifact itself (mtime, size), independent of the metric.
This inverts the naive instinct: you do not decide a reading is trustworthy by inspecting the reading; you decide
it by inspecting whether the source of the reading is quiescent. A falling disagree count is the most seductive
form of the trap because it mimics the shape of real progress; the defense is to refuse to interpret ANY count
from a file whose mtime is still moving, however encouraging the trend. Poll to quiescence first, then measure
once, then interpret.

The stakes: had I recorded "70-disagree regression (compound traps)" or even "33 and improving," I would have
reported a defect that did not exist — the settled reality was coverage rising with zero disagreement. A false
regression alarm on the compiler agent's in-flight M2 work would have been worse than useless: it would have
described the normal turbulence of climbing the runtime-compound cliff as breakage, exactly when the sibling was
doing the hardest and most valuable work. The loop's credibility depends on not crying regression at
work-in-progress, and the only reliable guard is settledness-of-the-artifact, not goodness-of-the-metric.

**The requirement it drove.** No corpus case, no ask — the finding is about the loop's own measurement
discipline. The output is this learning and the confirmed settled state (compiler.cdz quiescent at 198780 after
~60s stable, byte gate 136 agree / 0 disagree / 37 soft / 408 decline; the mid-edit 70 and 33 trap counts were
transient; M2 runtime-compound still declines cleanly, the +2 agree are scalar/const cases). General lesson:
**settledness is the artifact ceasing to change, established by polling its mtime/size to quiescence — NOT the
metric looking good or trending good; a converging count on a live file is the most dangerous mid-edit read
because it mimics a recovery trajectory and invites a false regression narrative, so refuse to interpret any
number from a still-changing artifact and poll to quiescence before measuring once.**
