(case "tl1 Set.to-list of a handler-built Int set: total order observable through indexing"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (- s 2))))
                (do
                  (def xs (Set.to-list (Set.insert (Set.insert (Set.insert (Set.of (list)) (St.a)) (St.a)) (St.a))))
                  (+ (* 100 (match (List.at xs 0) ((Option.Some v) v) ((Option.None) -99)))
                     (match (List.at xs 2) ((Option.Some v) v) ((Option.None) -99))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))
