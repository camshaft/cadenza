# Result verdict flip (2026-08-11)

Angle: the arm's Ok/Err verdict flipping between two IDENTICAL performs as
the state walks past the payload — the Result twin of op1's Option flip.
The n=3 seed puts the flip exactly between the two dispatches (Err then Ok
boundary case: 3<3 false -> Err 0, then 3<4 -> Ok 30).

GREEN x3:
- rsw1: 30030 / -99100 / -99970 (n=5/2/3 — Ok+Ok, Err+Err, Err+Ok)

Staged for the next 14c batch (with t2t1, nl2).
