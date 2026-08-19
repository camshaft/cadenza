(case "pysm1 probe: op takes an Option-typed ARGUMENT and the arm MATCHES it to choose the resume answer and next-state (Some carries an addend, None doubles the thread) — distinct from ops that RETURN Option"
  (input (do
  (effect O (op cmd (-> (Option Int64) Int64)))
  (def (main (: n Int64))
    (handle O (% n 3)
      ((cmd (m) s
        (match m
          ((Some x) (resume (+ s x) (+ s 1)))
          ((None) (resume s (* s 2))))))
      (+ (* 100 (O.cmd (Some 7))) (O.cmd (None)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 802 Int64))
  (call   main (: 0 Int64)) (output (: 701 Int64)))
