# Three-way mutual SCC x effects (2026-08-10)

Angle: the landed group multi-value fold covers a WIDTH-2 mutual SCC; nothing
pins width 3. Hunted a width-3 counterexample.

GREEN x3 (pin candidate):
- tw2: three-function mutual SCC — fa/fb tail-route by VALUE (not structural
  decrement), fc recurses the cycle, puts, and combines child + post-put draw.
  Width-3 extension of the exact landed idiom — 7/30.

DECLINE FENCE (staged, honest — "not yet reducible by the tail-resumptive fold"):
- tw1 + variants: every leg drawing PRE-recursion (2*(St.get) + fb(k-1)) declines
  at BOTH width 2 and width 3 — the width is NOT the discriminator; the
  pre-recursion non-tail draw in every leg is. Recurse-first let-bound child +
  post-draw combine (the landed shape) is the foldable form; draw-then-recurse
  in every leg of the cycle is the frontier.

The width-2 draw-then-recurse control declining too means the corpus's mutual
pin sits exactly on the foldable boundary. tw2 pins width-3 stays green.
