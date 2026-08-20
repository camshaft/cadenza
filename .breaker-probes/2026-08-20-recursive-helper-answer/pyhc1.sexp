(case "pyhc1 probe: the resume ANSWER is computed by a RECURSIVE top-level helper applied to the state — sumto x = x(x+1)/2 via self-recursion, so each tick resumes (sumto s) threading (+ s 1); a recursive helper call (not a simple arithmetic expr) sits in the answer position and must run to completion per dispatch while the state threads"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (sumto (: x Int64)) (if (<= x (: 0 Int64)) (: 0 Int64) (+ x (sumto (- x (: 1 Int64))))))
  (def (main (: n Int64))
    (handle E (+ (% n 3) (: 1 Int64))
      ((tick () s (resume (sumto s) (+ s 1))))
      (+ (* 1000 (E.tick)) (E.tick))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 3006 Int64))
  (call   main (: 0 Int64)) (output (: 1003 Int64)))
