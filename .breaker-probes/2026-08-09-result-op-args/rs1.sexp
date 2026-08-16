(case "rs1 a RESULT built from draw parity crosses dispatch as an op ARGUMENT — Ok scales with state, Err folds in negated"
  (input  (do
            (type Res (Ok Int64) (Err Int64))
            (effect E (op next (-> Int64)) (op judge (-> Res Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (judge (r) s (resume (match r
                                        ((Res.Ok v) (+ (* 100 v) s))
                                        ((Res.Err v) (- (- 0 v) s)))
                                      (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (+ (* 10 (E.judge (if (= (% d 2) 0) (Res.Ok d) (Res.Err d))))
                     (- (E.probe) n)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 4052 Int64))
  (call   main (: 3 Int64)) (output (: -68 Int64))
  (call   main (: -2 Int64)) (output (: -2008 Int64)))
