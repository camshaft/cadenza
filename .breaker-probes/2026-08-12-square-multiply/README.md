# Square-and-multiply state machine (2026-08-12)

Angle: the modular-exponentiation kernel as a handler — (base, acc) tuple
state where EVERY dispatch squares the base and 1-bits multiply the
accumulator, mod 1000. Computes 3^0b1010-ish per the driven bit-string;
a real algorithm shape with both fields transitioning per dispatch and the
transition depending on the OP ARGUMENT.

GREEN x3:
- sqm1: bits 1,0,1,n + a final read observing the n-bit — 243/323
  (weak-pin caught in draft AGAIN: the 4th bit's multiply lands after its
  read, so without the 5th read both seeds matched. The read-after rule:
  a bit's effect needs a LATER observation.)

Staged for the next 14c batch.
