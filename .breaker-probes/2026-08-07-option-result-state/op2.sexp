(case "op2 an Option RESUME value from a single-site arm — Some carries the excess, None answers the shortfall row"
  (input  (do
            (effect O (op get (-> Int64 (Option Int64))))
            (def (main (: n Int64))
              (handle O n
                ((get (k) s (resume (if (> k s) (Some (- k s)) (None)) (+ s 1))))
                (+ (match (O.get 10) ((Some d) d) ((None) -100))
                   (* 10 (match (O.get 0) ((Some d) d) ((None) -100))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -995 Int64))
  (call   main (: 20 Int64)) (output (: -1100 Int64)))
