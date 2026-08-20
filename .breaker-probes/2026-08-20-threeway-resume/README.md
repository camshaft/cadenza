# threeway-resume — three data-dependent resume sites (nested if on s%3)

## pymt2 — s%3: 0->(s*100,+1), 1->(s*10,+2), else->(s,+3). 4 dispatches, seed n%4.
n=1 s0=1: 10(s->3),300(s->4),40(s->6),600(s->7) => 950. n=0 s0=0: 0,10,300,40 => 350.
Verified 950/350 x3 + opt-sweep 0-div. Extends pyif1 (2-way) to a 3-way data-dependent
resume path; the fold reconverges three distinct resume calls (each own answer + state advance).
(match on integer literals of (% s 3) is NOT supported -> "not a scalar literal or _"; used nested if.)
