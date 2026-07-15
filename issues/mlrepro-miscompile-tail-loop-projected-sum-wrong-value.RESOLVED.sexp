;; ✅ FIXED (2026-07-14, seed rcdzc) — REGRESSION WITNESS. `main 0` now returns 5. The root was NOT
;; slot-aliasing as first hypothesized — it was a PERCEUS use-after-free in `backend/wasm/select.rs`
;; `binding_escapes`: `(. r 0)` projects a NESTED-COMPOUND child (the boxed sum `W.Atom`) OUT of the
;; `let`-bound tuple `r` and threads it into the recursive `loop` call as `last`. The escape analysis
;; only saw the SCALAR sibling projection `(. r 1)` (which copies its i64 out), judged `r` fully
;; borrowed, and DROPPED it after the projections — cascading to FREE the escaped boxed-sum child →
;; garbage read → 0 instead of 5. FIX: a nested-compound projection (`get_op(id) == None`) ESCAPES its
;; operand, so the aggregate is NOT reclaimed while its extracted child is still live. Migrated to the
;; corpus as `spec/semantics/10-bytes.sexp` ("a tail loop threading a projected boxed-sum accumulator
;; and a projected cursor decodes correctly") + unit test
;; `a_tail_loop_threading_a_projected_boxed_sum_accumulator_decodes_correctly`. Kept as the witness.
;;
;; ORIGINAL (2026-07-14): MISCOMPILE — SILENT WRONG VALUE. `cdz check` CLEAN, `cdz compile -t wasm`
;; SUCCEEDS (valid wasm), but the program returned the WRONG VALUE at run time. `main 0` must return 5
;; (the byte at position 0 of b"\x05\x07", wrapped in `W.Atom`, threaded once through the loop) but
;; returned 0.
;;
;; This was the SURVIVING face of the i32/i64 slot-aliasing loop-transform family. A sibling FIXED the
;; INVALID-WASM face (the minimal `miscompile-slot-alias-i32i64-loop-tupleproj.sexp` now compiles AND
;; returns the correct value), but the WRONG-VALUE face persisted — and a silent wrong value is more
;; dangerous than a loud validation error. (Hypothesis at filing was a loop-transform slot mis-read;
;; the actual root was the Perceus escape gap above.)
;;
;; SHARP BOUND (bisected 2026-07-14) — the JOINTLY-REQUIRED ingredients (remove ANY one → returns 5):
;;   (1) a SELF-TAIL loop that threads a BOXED SUM handle projected from a tuple (`(. r 0)`) as a param;
;;   (2) the loop ALSO advances its position from the OTHER projection of the SAME tuple (`(. r 1)`) —
;;       advancing via a recomputed `(+ pos 1)` instead (projecting only the sum) returns 5;
;;   (3) an `if` INSIDE the builder `one` that constructs the `(tuple <boxed-sum> pos)` — the branch need
;;       NOT be taken (here the taken branch is the `= …5` arm; a never-taken `if` still corrupts), so it
;;       is the loop-transform's slot ANALYSIS mis-slotting, not the executed path.
;; NOTE (broader than the original finding): the sum here is ALL-SCALAR (`Atom Int64 | Zero`) — a
;; COMPOUND-payload variant is NOT required for the wrong-value face (it WAS required for the old
;; invalid-wasm face). So the surviving bug is triggered by a plain boxed scalar sum.
;; CONTROLS (return 5): remove the `if` in `one`; advance pos via `(+ pos 1)`; or drop the sum and thread
;; a plain `Int64` (no boxing) — each in isolation compiles + runs correctly.
(do
  (type W (Atom Int64) (Zero))
  (def (one (: b Bytes) (: pos Int64))
    (if (= (Option.expect (Bytes.at b pos) "t") 5)
      (tuple ((. W Atom) (Option.expect (Bytes.at b pos) "v")) (+ pos 1))
      (tuple ((. W Atom) 99) (+ pos 1))))
  (def (loop (: b Bytes) (: n Int64) (: pos Int64) (: last W))
    (if (= n 0) last (let ((r (one b pos))) (loop b (- n 1) (. r 1) (. r 0)))))
  (def (wval (: s W)) (match s (((. W Atom) li) li) (((. W Zero) _) 0)))
  (def (main (: pos Int64)) (wval (loop b"\x05\x07" 1 pos ((. W Atom) 0))))
  (export main))

;; RESOLVED 2026-07-15 (trunk@1f3b3c348): file self-annotated ✅ FIXED (still-live-binding family closed).
