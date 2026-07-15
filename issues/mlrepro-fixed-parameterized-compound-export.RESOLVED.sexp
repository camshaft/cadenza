;; ✅ FIXED (by 2026-07-14, seed — landed by a sibling) — REGRESSION WITNESS. A PARAMETERIZED export
;; returning a COMPOUND now runs correctly: `cdz compile … && cdz-run pt.wasm --arg 5` →
;; `(: (tuple 5 6) (Tuple Int64 Int64))`. It used to TRAP "expected 1 argument(s), got 0" — the
;; resource-escape `make` did not forward the export's parameter at run time. Now the param threads
;; through `make` to the export body. Verified across the WHOLE family (all correct with `--arg 5` /
;; `--arg 1000000000`): tuple, record, List, BigInt (`1e9*1e9 = 1000000000000000000`), Result (`(Ok 5)`).
;;
;; ⚠ PROCESS NOTE (why this loop reported it OPEN for ~14 iterations): the per-iteration finding-check
;; rebuilds `cdz` (the compiler) but the fix landed in `cdz-run` (the runner) / runtime — a STALE
;; `cdz-run` kept reproducing the old trap. Finding-checks that RUN a component must
;; `cargo build --release --bin cdz-run` too, not only `--bin cdz`.
(do
  (def (pair (: n Int64)) (tuple n (+ n 1)))
  (def (main (: n Int64)) (pair n))
  (export main))
