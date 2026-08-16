(case "fa2 a TUPLE mixing Float64 and Int64 crosses as op ARGUMENT — the arm scales by the int"
  (input  (do
            (effect St (op scale (-> (Tuple Float64 Int64) Float64)))
            (def (main (: n Int64))
              (handle St 0.0
                ((scale (p) s (match p ((tuple f k) (resume (* f (Float64.of-int k)) s)))))
                (St.scale (tuple 2.5 (* n 2)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 25.0 Float64)))
