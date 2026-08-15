# kgt — Kadane tracker + a match-over-if decline face (2026-08-15, tick 1510)

- `kgt1.sexp` — Kadane max-subarray: feed extends-or-restarts via a four-way
  comparison lattice (2 nested ifs), best from a -99 sentinel. Seed flips the
  middle feed's sign (n-5): n=10 EXTENDS through +5 (peak 12), n=0 RESTARTS
  at -5 (peak 7). PASS ×3. **Pool — fills trn6/rpc1/kgt1 trio.**
- `kgt0-if-scrutinee-declines.sexp` — the SAME arm written with a match
  binder over an IF-expression scrutinee `(match (if pred a b) (c2 ...))` —
  DECLINES ×3 (clean todo). The binder-hoist idiom (rps2's workaround!) hits
  a wall when the scrutinee is an if-expression rather than a call/state
  compound. Wrinkle for the workaround's docs: hoisting works for arithmetic
  and call compounds, NOT for conditional expressions.

Flip-watch: kgt0 alongside the fence family (lstM/medK).
