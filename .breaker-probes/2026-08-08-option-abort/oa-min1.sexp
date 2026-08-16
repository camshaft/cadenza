(case "oamin1 a helper DEF whose match arm aborts — handler in the CALLER, no other effects"
  (input  (do
            (effect Bail (op out (-> Int64 Int64)))
            (def (unwrap (: o (Option Int64)) (: tag Int64))
              (match o
                ((Some v) v)
                ((None) (Bail.out tag))))
            (def (main (: n Int64))
              (handle Bail 0
                ((out (v) t (+ 500 v)))
                (+ (* 10 (unwrap (if (> n 0) (Some n) (None)) 11)) 3)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 43 Int64))
  (call   main (: -1 Int64)) (output (: 511 Int64)))
