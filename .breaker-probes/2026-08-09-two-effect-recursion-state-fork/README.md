# FINDING #14: TWO-EFFECT recursion state fork (tick 1028, base f2ab207de era, silent wrong value x3)

The #12 shape (post-recursion state fork) FIXED for single-effect recursion (147bd8ef4) but the
TWO-EFFECT face forks BOTH threads: a recursive callee drawing effects A AND B has its state
advances on BOTH threads discarded for the continuation - trailing (A.next)/(B.next) read
PRE-recursion state (ra3min2: 10 vs 15; ra3min3: 10 vs 12; steps also lost: 0x100).

- ra3min1: SINGLE-effect recursion + trailing draw -> PASS (the #12 fix holds for one effect)
- ra3min4: A-only recursion INSIDE the nested B handle -> PASS (nesting alone not the trigger)
- ra3min2/3: recursion draws BOTH; trailing A or B -> BOTH FORK
- ra3min5: direct-operand form (no let around the call) -> same FORK
- ra3: original 3-row witness (steps AND both trailing reads wrong)
- CONTRAST: ra1 (race result direct, NO trailing draws) and ra2 (race in SEED position) PASS -
  the fork only OBSERVABLE with post-recursion draws; the recursion's internal threading is right.
Trigger: [recursive callee performing TWO effects] x [any continuation draw after the call].
Adjacent to #12 (147dbd8ef4 fixed the one-effect face); the multi-effect out-state threading is the gap.
