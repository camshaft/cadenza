(case "oa3 the chained fetches FLATTENED — each Option is matched at top level via a helper, staying on the folding side"
  (input  (do
            (effect E (op fetch (-> (Option Int64))) (op probe (-> Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (unwrap (: o (Option Int64)) (: tag Int64))
              (match o
                ((Some v) v)
                ((None) (Bail.out tag))))
            (def (main (: n Int64))
              (handle E n
                ((fetch () s (resume (if (= (% s 2) 0) (Some s) (None)) (+ s 2)))
                 (probe () s (resume s s)))
                (+ (* 10 (handle Bail 0
                           ((out (v) t (+ 500 v)))
                           (let ((a (unwrap (E.fetch) 11)))
                             (let ((b (unwrap (E.fetch) 22)))
                               (* a b)))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 244 Int64))
  (call   main (: 1 Int64)) (output (: 5112 Int64))
  (call   main (: -2 Int64)) (output (: 4 Int64)))
