# Modulo toll over both captures (2026-08-19)

- `pyv3.sexp` — the toll is 100*(v % (s+1)): a NONLINEAR mix of both
  captures (7%2=1 then 5%3=2 for s0=1: 378; 7%1=0 then 5%2=1 for s0=0:
  167). Swapped operands or cross-frame pairs land wrong residues.
  Completes the toll-operator coverage: sum/product (commutative),
  difference (ordered), and now modulo (nonlinear + ordered + can
  ZERO a toll when the state divides the arg — the s0=0 first frame
  does exactly that). PASS x3 at 0c95d1a44.
