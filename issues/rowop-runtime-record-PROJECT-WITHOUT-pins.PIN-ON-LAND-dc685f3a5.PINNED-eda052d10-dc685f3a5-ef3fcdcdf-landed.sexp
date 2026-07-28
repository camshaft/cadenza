;; PIN-ON-LAND: add to spec/semantics/15-rows-and-open-sums.sexp (beside the l6 runtime Record.with case +
;; the eval-once value pin) once v-inference's Record.project/without-over-runtime-record slice (MR dc685f3a5,
;; stacks on 13fd27095 operand-materialize + 5cdc957da scope-skip) lands. On trunk 8a044187a these still
;; DECLINE 'a record row operation over a runtime record is not yet built' — HELD.
;;
;; Witness s-exprs handed by v-inference (their lib-test shapes, adapted to corpus (do …) form): a runtime
;; record via recursion-forced `mk` (the (+ v 987654321) operand arg is the eval-once probe — emits ONCE in
;; wasm, structurally asserted count==1 by v-inference's lib test; the corpus row only asserts VALUE). Both
;; keep ≥2 fields so the shared materialize_row_op_operand eval-once discipline is exercised. Includes the
;; preserved-first-field read (→1) AND a preserved-sibling read (→3) per v-inference's suggestion.
;;
;; ON LAND (dc685f3a5 on trunk): rebuild cdz, gate all 4 PASS on wasm+rust+rust-async, insert in 15-rows,
;; baseline (4 pass) x3, verify titles-agree/0-dup/0-omission + gate --check all 3 + roundtrip, commit + MR,
;; notify v-inference (matrix: with✅ project✅ without✅; merge/pop = next slice). Fold in the eval-once
;; VALUE pin (rowop-runtime-record-eval-once-pin, PIN-ON-13fd27095) as one matrix commit if both land together.

(case "a Record.project over a runtime record keeping two fields reads a kept field's value"
  (doc    "Row-op matrix (v-inference dc685f3a5): Record.project whose TARGET is a RUNTIME record (a
           recursion-forced `mk` result, not a literal) — used to decline 'not yet built'. Builds a fresh
           record from projections keeping {a,c}, materializing the operand ONCE (shared
           materialize_row_op_operand, the reviewer-49d6eec14 eval-once fix covers the whole family). Reads
           the kept first field `a` → 1.")
  (input  (do
            (def (mk (: n Int64)) (if (= n 0) (record (a 1) (b 2) (c 3)) (mk (- n 1))))
            (def (upd (: v Int64)) (. (Record.project (mk (+ v 987654321)) (a c)) a))
            (def (main (: v Int64)) (upd v))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64)))

(case "a Record.project over a runtime record reads a kept NON-FIRST field's value"
  (doc    "The preserved-sibling face: the SAME runtime Record.project keeping {a,c}, but reading the
           non-first kept field `c` → 3 — confirms a kept field other than the first also reads correctly
           through the materialized operand (a layout that miscounted a kept slot would misread it).")
  (input  (do
            (def (mk (: n Int64)) (if (= n 0) (record (a 1) (b 2) (c 3)) (mk (- n 1))))
            (def (upd (: v Int64)) (. (Record.project (mk (+ v 987654321)) (a c)) c))
            (def (main (: v Int64)) (upd v))
            (export main)))
  (call   main (: 1 Int64)) (output (: 3 Int64)))

(case "a Record.without over a runtime record dropping one field reads a surviving field's value"
  (doc    "Row-op matrix (v-inference dc685f3a5): Record.without over a RUNTIME record dropping {b}, keeping
           {a,c}, materializing the operand ONCE. Reads the surviving first field `a` → 1. The drop-shifts-
           layout twin of the project case; a without that recomputed layout from a source literal rather
           than the materialized operand would misread a slot after the drop.")
  (input  (do
            (def (mk (: n Int64)) (if (= n 0) (record (a 1) (b 2) (c 3)) (mk (- n 1))))
            (def (upd (: v Int64)) (. (Record.without (mk (+ v 987654321)) (b)) a))
            (def (main (: v Int64)) (upd v))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64)))

(case "a Record.without over a runtime record reads a surviving NON-FIRST field after the drop"
  (doc    "The surviving-sibling face: the SAME runtime Record.without dropping {b}, reading the surviving
           `c` → 3 — after dropping the MIDDLE field, the last field must still read correctly through the
           materialized operand (the drop shifts c's position; a stale layout would misread it).")
  (input  (do
            (def (mk (: n Int64)) (if (= n 0) (record (a 1) (b 2) (c 3)) (mk (- n 1))))
            (def (upd (: v Int64)) (. (Record.without (mk (+ v 987654321)) (b)) c))
            (def (main (: v Int64)) (upd v))
            (export main)))
  (call   main (: 1 Int64)) (output (: 3 Int64)))
