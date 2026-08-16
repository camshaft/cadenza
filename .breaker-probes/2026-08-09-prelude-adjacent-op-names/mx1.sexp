(case "mx1 ops named after PRELUDE-adjacent words — max and len as op names dispatch qualified without collision"
  (input  (do
            (effect E (op max (-> Int64)) (op len (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((max () s (resume s (+ s 1)))
                 (len () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (* 10 (+ (E.max) (E.len))) (- (E.probe) n))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 92 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64))
  (call   main (: -3 Int64)) (output (: -48 Int64)))
