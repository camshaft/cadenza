(case "pyid1 probe: the INNER handler's arm itself draws from the OUTER effect (legal: outer encloses inner) — each inner I.now resumes its state plus a fresh outer O.get, so the two handler states thread independently and the inner answer folds in an outer sub-draw taken at the outer's current state"
  (input (do
  (effect O (op get (-> Int64)))
  (effect I (op now (-> Int64)))
  (def (main (: n Int64))
    (handle O (+ (% n 3) (: 5 Int64))
      ((get () s (resume (* s 10) (+ s 1))))
      (handle I (: 2 Int64)
        ((now () t (resume (+ t (O.get)) (+ t 1))))
        (+ (I.now) (* 1000 (I.now))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 73062 Int64))
  (call   main (: 0 Int64)) (output (: 63052 Int64)))
