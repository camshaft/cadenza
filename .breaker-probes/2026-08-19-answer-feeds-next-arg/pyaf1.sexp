(case "pyaf1 probe: a DATA-DEPENDENCY CHAIN across dispatches — each step(x) resumes x+s and the result feeds the NEXT dispatch's argument, so the three dispatches form a fold where the arg thread and the handler-state thread advance independently"
  (input (do
  (effect E (op step (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((step (x) s (resume (+ x s) (+ s 1))))
      (let ((a (E.step 1)))
        (let ((b (E.step a)))
          (let ((c (E.step b)))
            (+ (* 100 c) (+ (* 10 b) a)))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 742 Int64))
  (call   main (: 0 Int64)) (output (: 421 Int64)))
