(case "oa1 an Option-returning op's None arm aborts LOCALLY — the local bail-out in a match arm homes correctly"
  (input  (do
            (effect E (op fetch (-> (Option Int64))) (op probe (-> Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((fetch () s (resume (if (= (% s 2) 0) (Some s) (None)) (+ s 3)))
                 (probe () s (resume s s)))
                (+ (* 100 (handle Bail 0
                            ((out (v) t (+ 500 v)))
                            (match (E.fetch)
                              ((Some v) (* 10 v))
                              ((None) (Bail.out 77)))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 4003 Int64))
  (call   main (: 1 Int64)) (output (: 57703 Int64))
  (call   main (: -2 Int64)) (output (: -1997 Int64)))
