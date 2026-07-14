;; MISCOMPILE — SILENT WRONG VALUE (2026-07-14). `cdz check` CLEAN, `cdz compile -t wasm` SUCCEEDS
;; (valid wasm), but the program returns the WRONG VALUE at run time. `main 0` must return 5 (the byte
;; at position 0 of b"\x05\x07", wrapped in `W.Atom`, threaded once through the loop) but returns 0.
;;
;; This is the SURVIVING face of the i32/i64 slot-aliasing loop-transform family. A sibling FIXED the
;; INVALID-WASM face (the minimal `miscompile-slot-alias-i32i64-loop-tupleproj.sexp` now compiles AND
;; returns the correct value), but the WRONG-VALUE face persists — and a silent wrong value is more
;; dangerous than a loud validation error. Root still `backend/wasm/select.rs` loop-transform emit: a
;; tuple-projected value threaded into a loop param slot is read from the wrong slot.
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
