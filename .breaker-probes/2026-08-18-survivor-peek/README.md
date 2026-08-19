# Passive peek reads the survivor's state (2026-08-18)

- `pyx1.sexp` — tick double-replays (discarded +1, survivor *3); a
  separate PASSIVE op (peek, no state advance) then reads the thread.
  The peek must see the survivor's TRIPLED state, not the discarded
  increment nor the pre-replay value (311 = 11 + 100*3 for s0=1; the
  s0=0 seed zeroes the tripled thread: 10 + 0). Multi-OP variant of
  dbr6's survivor-thread law: the observer is a different op with its
  own arm, not a second tick. PASS x3 at b7972ffd6.
- `pyx2.sexp` — a POKE overwrites the thread from its ARGUMENT: answers
  the OLD state, replaces the thread wholesale (s := 7); the next tick
  reads the transplanted value (70030 / 70010). Merge-instead-of-replace
  or answer-the-new-value shift separate ranges. With pyx1 (passive
  read) this covers the observer/mutator op pair over one thread. PASS
  x3 at 0c95d1a44.
