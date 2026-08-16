# Mixed-arity op battery (2026-08-10) — one handler, ops at arity 0/1/2/3

Angle: the five-arg pool (q5/q6) covers WIDE single-op rows; this bank covers a HANDLER
with heterogeneous arities where results chain into later calls' argument rows.

All GREEN x3 (wasm/rust/rust-async), hand-modeled in python before gating:
- m1: arity 0/1/2/3 chained r0->r1->r2->r3, each op advances state differently (+1/+10/+100/+1000) — 203665/154421
- m2: a ZERO-arg perform INSIDE a three-arg op's argument row (draw order within the row) — 7514/5712
- m3: recursive walk where the two-arg op consumes k + a fresh z-draw per hop — 6335/4928
  (first draft used mod 5: NON-TERMINATING — d advances +6/hop, mod-5 residue cycle never
  hits 0; the python hand-model hung and caught it pre-gate. mod 7 terminates.)

No counterexample. Pin candidates alongside q5/q6.
