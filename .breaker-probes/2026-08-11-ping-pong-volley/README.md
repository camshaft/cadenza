# Ping-pong volley (2026-08-11)

Angle: strict A->B->A alternation where each effect's ANSWER becomes the
other's ARGUMENT through a recursive driver — the il (interleaving) pins
alternate draws but don't thread answers into arguments cross-effect.

GREEN x3:
- pp1: volley(n, ball) — ball = B.pb(A.pa(ball)) per hop; A strides +1,
  B +10, both states independent — 752/1
- pp2: the INNER handler re-instantiated per hop (fresh B shadow seeded by
  the loop counter k*100 wraps each exchange; handle-in-recursive-arg-position)
  — 1712/1

Pin candidates: 250 pool.
