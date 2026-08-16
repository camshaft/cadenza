# Boyer-Moore majority vote (2026-08-12)

Angle: the classic majority-vote automaton as a handler — (leader, votes)
with a THREE-WAY transition (same-candidate increment / depose-on-zero /
decrement), a nested-if transition where the taken path depends on BOTH
state fields and the argument. Seed 7 reinforces the early leader; seed 9
walks through depose (7,7,9,9,9: votes 1,2,1,0->depose... model says 9 wins).

GREEN x3:
- bmv1: 7/9 (the winner flips by seed)

Staged: 14c pool at 13 — two+ batches ready.
