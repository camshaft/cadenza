# OPEN QUESTION: shadow whose INIT draws the shadowed effect + tolls (2026-08-18)

pysh3 — inner handle over the SAME effect whose INIT performs (E.tick) on
the outer arm, BOTH arms tolled. UNIFORM x2 backends (wasm AND rust agree)
but diverges from my hand model:

- red2 (inner body = pure constant 7, init still draws outer): actual 2007
  = 7 + outer toll 2000 — MATCHES the standard deep-handler model exactly.
- red1 (inner body = one shadowed E.tick): actual 7010; my model says 3010
  (inner 10 + inner toll 1000 + outer toll 2000). Divergence = exactly
  +4000 at n=10; pysh3 full form diverges 41010 vs modeled 26010 (n=10)
  and 27000 vs 13000 (n=0).

So the divergence appears only when the inner region BOTH has a shadowing
draw AND the init drew the outer arm. Uniform => not a differential
miscompile; either intended (which semantics?) or a uniform bug. ASKED
v-effects (owns fold semantics) before pinning any oracle. NOT promotable
until resolved. pysh1/pysh2 (models matched, corpus-bound) unaffected.
