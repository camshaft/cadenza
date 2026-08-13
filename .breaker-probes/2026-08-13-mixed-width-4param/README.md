# 2026-08-13 mixed-width 4-param op (tick 1386)

- `mx4.sexp` — op `(-> Int64 UInt8 Bool Int64 Int64)`: a NARROW UInt8 slot rides
  BETWEEN wide Int64 slots with a Bool flag routing the arm (flag doubles arg 1;
  parity flips which dispatch doubles). mx1 (14b) covers 3-arg Int64/String/Bool;
  the narrow-slot-between-wides marshal at 4 params is the new face (UInt8
  widened via Int64.of in the arm). PASS ×3 (5130509/5090515).
- `gpd1.sexp` — PERFORMING match guard: CDZ0407 decline ×3 — guards must be
  side-effect-free BY DESIGN (purity policy; the gd/gp families pin the allowed
  pure side, diagnostics family pins the reject). NOT a todo to watch — spec'd
  behavior; kept as the decline witness beside the mx4 pin. My draft also hit
  the guard SYNTAX (it's `(guard v pred)`, not `((v pred))` — CDZ0101 unbound).
