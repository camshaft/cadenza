(case "oamin3 the helper argument is an OP RESULT (E.fetch) instead of a pure if"
  (input  (do
            (effect E (op fetch (-> (Option Int64))))
            (effect Bail (op out (-> Int64 Int64)))
            (def (unwrap (: o (Option Int64)) (: tag Int64))
              (match o
                ((Some v) v)
                ((None) (Bail.out tag))))
            (def (main (: n Int64))
              (handle E n
                ((fetch () s (resume (if (= (% s 2) 0) (Some s) (None)) (+ s 2))))
                (+ (* 10 (handle Bail 0
                           ((out (v) t (+ 500 v)))
                           (unwrap (E.fetch) 11)))
                   3)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 43 Int64))
  (call   main (: 1 Int64)) (output (: 5113 Int64)))
