# three-level-same-shadow — three handlers of the SAME effect E, nested, each shadows

## pysh7 — outer(*1000,seed n%3) > mid(*100,seed 5) > inner(*10,seed 20), all +1 thread
Body: outer#1 + (mid#1 + (inner#1 + inner#2)).
Model: outer#1 = sO0*1000; mid#1 = 500; inner#1 = 200 (sI->21); inner#2 = 210. sum = 1000*sO0 + 910.
n=10 1910, n=0 910. Verified x3 + opt-sweep 0-div.
Extends pysh4 (2-level same-effect shadow) to 3 levels: each nested handle E shadows the
outer ones only in its own body, each threads its own state. All three coexisting E handlers
resolve to the lexically-innermost enclosing handler per E.tick.
