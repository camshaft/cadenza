# try × effects probes — BLOCKED behind operator-gated BRICK 3b (2026-08-26)

tye1 (Result-returning op unwrapped via `?` in a handled body, Ok-sum + Err-short-circuit) and
tye2 (Option twin, None short-circuits past the second dispatch) both DECLINE on wasm+rust.
tye3 CONTROL proves it is NOT effects-specific: a `?` over a plain RUNTIME Result param (no
effects) declines identically — the general runtime-discriminant `?` boundary = BRICK 3b,
operator-gated (per the try-operator lane's standing note: "do NOT touch uninvited").

NOT FILED (known, gated, already 2 baseline todos in the class). These probes become auto-flip
verification witnesses if/when the operator un-gates brick 3b: tye1 70/-1, tye2 7/-1, tye3 42.
