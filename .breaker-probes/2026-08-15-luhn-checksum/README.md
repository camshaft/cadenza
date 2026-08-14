# lhn1 — Luhn checksum accumulator (2026-08-15, tick 1497)

(sum, position) state: `feed` doubles every second digit by position parity,
subtracting 9 from two-digit doubles (the Luhn rule); `chk` answers 1 on a
multiple of 10, else the residue. The seed digit (n%7: 3 vs 0) doubles on both
runs, but only n=10's crosses the subtract-nine threshold (3*2=6 stays, 0*2=0
stays — actually neither crosses; the DIVERGENCE is in the values: rows
4,10,19,24,4 vs 4,4,13,18,8 — every row after the seed feed differs, and the
final checks land on different residues).

F24-safe by design: 5 dispatches, 3-branch nested-if over a 2-tuple — inside
the known-safe envelope (the F24 zone starts at ~6 dispatches x multi-branch
x 3+-tuple).

PASS ×3 wasm. **Pool.**
