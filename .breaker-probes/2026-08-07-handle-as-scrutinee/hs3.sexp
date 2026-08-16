(case "hs3 an inner SAME-effect handle's result is the IF condition — the taken branch re-performs against the outer"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (if (> (handle St 10
                         ((next () t (resume t (+ t 5))))
                         (+ (St.next) (St.next)))
                       (St.next))
                    (+ 100 (St.next))
                    (- 0 (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106 Int64))
  (call   main (: 50 Int64)) (output (: -51 Int64)))
