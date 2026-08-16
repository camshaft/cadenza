# Tuple-to-tuple transformer (2026-08-11)

Angle: an op whose ARGUMENT and RESULT are both tuples — the compound-in/
compound-out crossing chained twice, with the arm swapping components and
salting from the state. (Landed tuple pins cross one direction per op;
the both-directions transformer was uncovered.)

GREEN x3:
- t2t1: swap+salt chained twice — 4020/1020
  (id chosen via the new FREE-ID PRE-CHECK — tt1 was taken in 14c.)

Staged for the next 14c batch.
