(case "hb1 a BOOL op argument computed by comparing a draw's residue — the arm branches on the delivered flag against live state"
  (input  (do
            (effect E (op next (-> Int64)) (op judge (-> Bool Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (judge (f) s (resume (if f (+ 100 s) (- 0 s)) (+ s 5))))
                (do (E.next)
                    (+ (E.judge (= (% (E.next) 3) 0)) (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 119 Int64))
  (call   main (: 2 Int64)) (output (: 113 Int64))
  (call   main (: 0 Int64)) (output (: 5 Int64))
  (call   main (: -4 Int64)) (output (: 101 Int64)))
