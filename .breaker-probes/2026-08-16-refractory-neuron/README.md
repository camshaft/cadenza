# rfr1 — integrate-and-fire neuron with refractory period (2026-08-16, tick 1601)

Attack: NESTED-IF in the arm (refractory gate outside, threshold crossing inside)
with the compound `(+ pot (+ x (% n 3)))` repeated THREE times across the inner
if's scrutinee and both branches — shared-subexpression pressure in the
F24-adjacent shape, but at a safe envelope (3 branches, 5 dispatches, 3-tuple).
First neuron/spike-train theme in the corpus.

Differential: the seed bias (n%3) shifts WHICH excitation crosses threshold.
n=10 (bias 1): crossing at excite#2 (pot 5+5+1=11), refractory swallows #3,#4,
pot restarts and #5 lands at 9 sub-threshold. n=0 (bias 0): crossing at #3
(4+5+3=12), refractory swallows #4,#5 — the fire count ends 1 in BOTH runs but
the row patterns are disjoint, so a collapsed-fire-count shortcut can't fake it.
Refractory rows answer fires*100+55 with the potential FROZEN at zero — a
branch whose answer reads a *different* state field than the one it decrements.

Hand model (python, banked in transcript):
- n=10: rows [5,117,155,155,9] + fires 1 → 5117155155009001 (base-1000 packing)
- n=0:  rows [4,9,127,155,155] + fires 1 → 4009127155155001

Pass ×3 wasm on trunk 5c8e8e9a3; rust + rust-async gated under load storm.
