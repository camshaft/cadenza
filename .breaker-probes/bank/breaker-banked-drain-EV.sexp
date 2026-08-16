(case "a loop-invariant HEAP construction inside a fold is shared or re-built but never corrupted"
  (doc    "The CONSTRUCTION-site face of the LICM-heap family (the :2524/:16170 pins consume a heap
           child EXTRACTED from a threaded invariant; here the invariant heap value is a LITERAL
           BUILT in the loop body): `(let ((probe (list 5 6 7))) …)` allocates per iteration (or is
           soundly hoisted with per-iteration retains) and the body BORROWS it via a cycling
           List.at read — 6 iterations sum 5+6+7 twice = 36; zero iterations = 0. Whether the
           optimizer hoists the invariant literal or rebuilds it, the observable must be a pristine
           3-element list every iteration (a hoist-with-one-retain that lets a later iteration read
           a corrupted or freed probe drifts the sum; the empty-loop control pins the hoisted
           construction doesn't leak or trap when never entered).")
  (input  (do
            (def (go (: i Int64) (: n Int64) (: acc Int64))
              (if (= i n)
                acc
                (let ((probe (list 5 6 7)))
                  (go (+ i 1) n (+ acc (Option.expect (List.at probe (% i 3)) "in range"))))))
            (def (main (: n Int64))
              (go 0 n 0))
            (export main)))
  (call   main (: 6 Int64)) (output (: 36 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
