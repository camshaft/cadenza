# rps — RPS judge: in-branch recompute explodes, match-binder hoist SAVES it (2026-08-15, tick 1506)

Rock-paper-scissors vs a hidden LCG opponent: play advances the LCG, judges
by the mod-3 cyclic-difference rule (+1/0/-1), score packs (wins, losses).
Seeds steer the opponent: n=10 mostly loses (negative packed total
-100010097), n=0 mostly wins (100009921).

- `rps1-explodes.sexp` — the compound LCG expression `(% (+ (* seed 5) 3) 16)`
  recomputed in EVERY branch of the 3-branch arm × 4 dispatches → INVALID
  WASM ×3: 2,506,487-byte emit, 'too many locals' (LOCALS kind).
- `rps2.sexp` — SAME protocol, the LCG advance and the mod-3 move hoisted
  through irrefutable match binders (s2, o), branches only compare/resume →
  PASS ×3. **Pool.**

Fifth F24 hit, and the tightest same-protocol pair yet: identical answers,
identical branch count, identical dispatches — the only delta is in-branch
recompute of a compound expression vs binder hoist. This is the USER-VISIBLE
workaround (hoist via match binder) AND the fix's before/after in one bank.
Also note: 4 dispatches through the branching arm (under the previous ≤4-safe
line) still exploded when each branch recomputes the compound — the envelope
is (dispatches × per-branch compound recomputes), not dispatches alone.
