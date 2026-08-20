# conditional-resume-both-arms — two distinct resume sites in an if, data-dependent path

## pyif1 — even s: resume(s*100, s+1); odd s: resume(s*10, s+3). 3 dispatches.
Seed n%3. n=10 s0=1: odd 10(s->4), even 400(s->5), odd 50(s->8) => 460.
n=0 s0=0: even 0(s->1), odd 10(s->4), even 400(s->5) => 410.
Verified 460/410 x3 + opt-sweep 0-div. The fold handles TWO resume calls (distinct answer AND
next-state per branch) reconverging out of the if — the resume site is not unique in the arm,
and the state advance is data-dependent (+1 vs +3). Distinct from prior single-resume-site arms.
