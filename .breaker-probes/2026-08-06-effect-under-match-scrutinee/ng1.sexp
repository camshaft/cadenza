(case "ng1 NEGATIVE values thread every effect slot — state, argument, and result stay signed"
  (input  (do
            (effect St (op dip (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St -100
                ((dip (v) s (resume (+ v s) (- s 10))))
                (+ (St.dip (- 0 n)) (St.dip 3))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -212 Int64)))
