;; PIN-ON-LAND: add to spec/semantics/15-rows-and-open-sums.sexp once v-inference's Record.merge/pop-over-
;; runtime-record slice (MR ef3fcdcdf, stacks on project/without dc685f3a5 + with-operand 13fd27095 +
;; scope-skip 5cdc957da) lands. On trunk 8a044187a these still DECLINE 'not yet built' — HELD. This COMPLETES
;; the row-op-over-runtime-record matrix (with/project/without/merge/pop all build, none re-emit the operand).
;;
;; Witness s-exprs handed by v-inference (adapted to corpus (do…) form). Per-operand distinctive constants
;; (+ v 987654321 / +v 111222333) are eval-once probes structurally guarded by v-inference's lib tests
;; (each emits 1× in wasm); corpus rows assert VALUE only.
;;
;; ON LAND (ef3fcdcdf on trunk): rebuild cdz, gate both PASS on wasm+rust+rust-async, insert in 15-rows,
;; baseline (2 pass) x3, verify titles-agree/0-dup/0-omission + gate --check all 3 + roundtrip, commit + MR,
;; notify v-inference (matrix COMPLETE). Ideally land TOGETHER with the eval-once value pin + project/without
;; pins as ONE 15-rows matrix commit if 13fd27095+dc685f3a5+ef3fcdcdf are all on trunk at pin time.

(case "a Record.pop over a runtime record splits off the named field's value"
  (doc    "Row-op matrix (v-inference ef3fcdcdf): Record.pop over a RUNTIME record (recursion-forced `mk`)
           splits it into (popped-value, rest) at field `a` — was a decline 'not yet built'. Materializes
           the operand ONCE (shared materialize_row_op_operand). Reads tuple element 0 (the popped `a`) → 1.")
  (input  (do
            (def (mk (: n Int64)) (if (= n 0) (record (a 1) (b 2) (c 3)) (mk (- n 1))))
            (def (upd (: v Int64)) (. (Record.pop (mk (+ v 987654321)) a) 0))
            (def (main (: v Int64)) (upd v))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64)))

(case "a Record.merge of two runtime records reads a field from the second operand"
  (doc    "Row-op matrix (v-inference ef3fcdcdf): Record.merge unions two disjoint RUNTIME records (both
           recursion-forced) — was a decline 'not yet built'. Each operand materializes ONCE (distinct
           eval-once probe constants per operand, structurally guarded by the lib test). Reads `c` from the
           SECOND operand `(mkB …)` = {c,d} → 3 — confirms a field from the second operand's row survives the
           union at its correct slot.")
  (input  (do
            (def (mkA (: n Int64)) (if (= n 0) (record (a 1) (b 2)) (mkA (- n 1))))
            (def (mkB (: n Int64)) (if (= n 0) (record (c 3) (d 4)) (mkB (- n 1))))
            (def (upd (: v Int64)) (. (Record.merge (mkA (+ v 987654321)) (mkB (+ v 111222333))) c))
            (def (main (: v Int64)) (upd v))
            (export main)))
  (call   main (: 1 Int64)) (output (: 3 Int64)))
