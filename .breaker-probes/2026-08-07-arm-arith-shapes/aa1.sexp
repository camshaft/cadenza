(case "aa1 the resume VALUE is a deep pure expression over the op arg AND state — (v+s)^2 - v*s per dispatch"
  (input  (do
            (effect E (op f (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((f (v) s (resume (- (* (+ v s) (+ v s)) (* v s)) (+ s v))))
                (+ (E.f 3) (E.f 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 133 Int64))
  (call   main (: 0 Int64)) (output (: 28 Int64))
  (call   main (: -1 Int64)) (output (: 19 Int64)))
