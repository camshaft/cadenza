(case "em2 an EMPTY-Map handler state queried before ANY advance, then grown from empty"
  (input  (do
            (effect St (op q (-> Int64 Int64)) (op put (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle St Map.empty
                ((q (k) s (resume (match (Map.lookup s k) ((Some v) v) ((None _u) -7)) s))
                 (put (k) s (resume 0 (Map.insert s k (* k 10)))))
                (+ (* 100 (St.q a))
                   (+ (* 0 (St.put a)) (St.q a)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: -670 Int64)))
