# 2026-08-13 drawn-exponent modexp (tick 1419)

- `dxb1.sexp` — the INVERSE of sqm1: there the BODY supplies bits as op args and
  the ARM squares/multiplies; here the ARM peels the threaded exponent (s%2, s/2)
  as a bit stream and the BODY's recursive square-and-multiply consumes it
  LSB-first (result and power both threaded through the recursion params, mod
  101). 3^5=41, 3^12=80 mod 101 — full modexp verified end-to-end with the
  exponent living behind the effect boundary. PASS ×3.
