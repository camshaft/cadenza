# perform-in-helper — a helper fn called from the body performs the effect

## pynd5 — (twice) = tick + tick, called from the handle body; body = twice() + 1000*tick
tick answers s*10 threads s+1, s0=n%3. Order tick#1(s0), tick#2(s0+1), tick#3(s0+2).
sum = 10*s0 + 10*(s0+1) + 1000*10*(s0+2) => n=10 30030, n=0 20010.
Verified 30030/20010 x3 + opt-sweep 0-div. The two ticks inside `twice` originate in a
separate function frame but route to the enclosing handler and thread state in call order —
distinct from a perform lexically in the body. (The RESUME stays tail in the arm, so this
folds; contrast a cross-function RESUME which is the deferred-increment decline.)
