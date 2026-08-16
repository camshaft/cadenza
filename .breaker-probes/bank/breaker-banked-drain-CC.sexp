(case "reversed and negative slice bounds over a runtime rope yield None at run time"
  (doc    "The RUNTIME companion of the const slice-bounds family (:1850ff — those fold before emit;
           this runs the heap bounds check over a 2-chunk rope with PARAM indices). Three faces per
           call: a param-driven window (100s digit: (1,4)=\"ell\" Some → 1; (4,2) reversed → 0), the
           reversed literal pair (10s: always None), the negative start (1s: always None) → 100 then
           0. A runtime bounds check that clamped instead of rejecting, or compared signed so -1
           passes, or off-by-one'd the reversed test, flips a digit — over a rope where the check must
           run BEFORE any seam-crossing byte-extent mapping (:3893's in-range seam pin is the
           complement).")
  (input  (do
            (def (chk o) (match o ((Some s) 1) ((None u) 0)))
            (def (main (: st Int64) (: en Int64))
              (let ((s (String.concat "hel" "lo")))
                (+ (* 100 (chk (String.slice s st en)))
                   (+ (* 10 (chk (String.slice s 3 1)))
                      (chk (String.slice s -1 3))))))
            (export main)))
  (call   main (: 1 Int64) (: 4 Int64)) (output (: 100 Int64))
  (call   main (: 4 Int64) (: 2 Int64)) (output (: 0 Int64)))
