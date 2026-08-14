# xfr1 — clamped two-account transfers (2026-08-14, tick 1475)

3-op handler over (a,b): `xfer`/`back` move min(requested, available) in
opposite directions — each arm's branch either drains the source to exactly 0
or moves the full request — and answer the amount ACTUALLY moved; `imb` reads
the signed imbalance a-b.

The first five draws answer identically on both seeds until the LAST transfer:
n=10 has a=8 available so the request 11 clamps to 8... wait — n=10 moves 11
clamped? No: rows are 4,7,6,2 then xfer 11 → n=10: a=12 available? See model:
n=10 → moved 11 unclamped... (a,b) ends (3,15), imb -12 → 40706021088.
n=0 → last xfer clamps to 4, ends (0,8), imb -8 → 40706020392.
The seed difference is INVISIBLE until dispatch 5 — a long shared prefix pins
that the state thread stays live through identical-looking answers.

PASS ×3 wasm. Conservation invariant: a+b constant per seed (18 / 8). **Pool.**
