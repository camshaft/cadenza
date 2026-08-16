# STALE SUM-PAYLOAD BINDER across dispatches (found tick 1010, base 45773f3ab)

SILENT WRONG VALUE x3 backends (wasm, rust, rust-async agree on the WRONG answer = frontend/fold bug).

cmmin5 minimal: arm = (match c ((Cmd.Go k) (match s ((Mode.Idle) (resume k ...)) ((Mode.Run j) (resume (+ j k) ...)))))
Two dispatches: (M.step (Cmd.Go 15)) then (M.step (Cmd.Go 7)).
Expected 15 + (15+7) = 37. Got 45 = 15 + (15+15): the SECOND dispatch's `k` still reads the FIRST call's payload.

Bisection:
- cmmin5 = MINIMAL FAIL: [SUM op-arg matched OUTER] x [SUM state matched INNER] x [payload k read in inner Run branch]
- cmmin4: nesting FLIPPED (state outer, arg inner) -> PASS. Ordering matters.
- cmmin7: inner match on SCALAR state via literal patterns -> PASS. Inner SUM scrutinee required.
- cmmin6: inner IF instead of match -> PASS.
- cmmin2: sum arg + scalar state (no inner match) -> PASS.
- cmmin3: scalar arg + sum state (single match) -> PASS.
- cm1/cmmin1: the original 4-op and 2-op cross-product faces (fail, 41/45).
Trigger: [sum ARG match outer] x [sum STATE match inner] x [arg payload binder read inside the inner sum-match branch].
The payload binder k is captured/copied ONCE (first dispatch) in the fold's arm re-instantiation and not refreshed per dispatch.

## Scope extension (tick 1011)
- cm-opt: STD Option payload -> FAIL (45 vs 37). Not user-sum-specific.
- cm-tup: TUPLE payload destructured in inner branches -> FAIL (51 vs 41). Compound payloads too.
- cm-2ops: dispatches through DIFFERENT ops with identical arm shapes -> FAIL (45 vs 37).
  step2's k read step's payload => staleness crosses ARM boundaries; the frozen environment is shared
  across arm instantiations, not per-arm. Consistent with deep_fresh_copy freezing the outer-match
  binder env at first dispatch.
