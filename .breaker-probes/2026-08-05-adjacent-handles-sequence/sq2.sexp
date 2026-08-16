(case "sq2 the FIRST handle's result SEEDS the second (sequential state handoff between handle instances)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def r1 (handle St n ((a (u) s (resume s (+ s 5)))) (+ (* 0 (St.a)) (St.a))))
                (handle St r1 ((a (u) s (resume s (* s 2)))) (+ (St.a) (St.a)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 24 Int64)))
