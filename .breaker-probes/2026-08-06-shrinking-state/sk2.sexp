(case "sk2 a SET state churns — inserts and removes interleave across dispatches, canonical at each read"
  (input  (do
            (effect St (op flip (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (Set.of (list 1 2 3))
                ((flip (k) s
                  (resume (Set.len (if (Set.contains s k) (Set.remove s k) (Set.insert s k)))
                          (if (Set.contains s k) (Set.remove s k) (Set.insert s k)))))
                (+ (* 100 (St.flip 2))
                   (+ (* 10 (St.flip 2))
                      (St.flip 9)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 234 Int64)))
