(case "pyres1 probe: a RESULT-typed resume answer with parity-selected payload — step answers (Ok (* s 10)) on even state and (Err (+ s 100)) on odd, threading (+ s 1); the body folds Err to its negation, so the two dispatches cross the Ok/Err boundary and a sum-type answer with a varying payload rides the resume seam per dispatch"
  (input (do
  (effect E (op step (-> (Result Int64 Int64))))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((step () s (resume (if (= (% s 2) (: 0 Int64)) (Ok (* s 10)) (Err (+ s 100))) (+ s 1))))
      (+ (* 1000 (match (E.step) ((Ok v) v) ((Err e) (- (: 0 Int64) e))))
         (match (E.step) ((Ok v) v) ((Err e) (- (: 0 Int64) e))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: -100980 Int64))
  (call   main (: 0 Int64)) (output (: -101 Int64)))
