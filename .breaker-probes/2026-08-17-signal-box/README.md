# sgb1 — signal box with interlocked levers (2026-08-17, tick 1714)

Attack: a MUTUAL INTERLOCK — each op's guard reads the field the OTHER op
sets (lever needs sig==0, pull needs pts==1), so the reachable-state graph
has a deadlock-shaped corner: clearing the signal LOCKS the lever until...
nothing in this protocol drops the signal, so a cleared box refuses every
later lever (an absorbing interlock, reached deliberately by n=10). cwx1's
ordering gate escalated to bidirectional.

Differential: points pre-set (1) vs not: n=10 clears at once (700) then
every lever is INTERLOCKED (811, 811 — the absorbing corner) and the repeat
pull re-answers 700 idempotently (signal already up, rebuild same); n=0
refuses the first pull (900), throws (11), clears (701), interlocks (811).
Reads 110 vs 111 (moves 0 vs 1!).

Hand model: n=10 → 7008117008110110; n=0 → 9000117018110111 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk 141665bdd. (id sgb1; rsw1
candidate name was taken — grep read.)
