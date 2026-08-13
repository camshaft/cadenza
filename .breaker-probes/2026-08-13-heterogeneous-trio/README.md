# 2026-08-13 heterogeneous result-type trio (tick 1387)

- `het1.sexp` — ONE effect, THREE ops with THREE result types (Int64/Bool/String)
  sharing one scalar thread, interleaved in one run and consumed by type-specific
  idioms (arith, if, byte-len). Per-op result-type diversity exists across CASES
  (Symbol/Bool joins noted in 14b); a single effect mixing three result KINDS
  through one state thread in one run was unpinned. Arm-performs-own-sibling was
  coverage-killed (CDZ0401 by design, witness noted at 14c:2087). Seeds route the
  parity/mod-3 branches differently (614012/802014). PASS ×3.
