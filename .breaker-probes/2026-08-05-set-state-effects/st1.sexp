(case "st1 a SET handler state accumulating perform args with DEDUP observable via resume values"
  (input  (do
            (effect St (op add (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle St (Set.of (list))
                ((add (v) s (resume (Set.len s) (Set.insert s v))))
                (+ (* 100 (St.add a))
                   (+ (* 10 (St.add a))
                      (St.add (+ a 1))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 11 Int64)))
