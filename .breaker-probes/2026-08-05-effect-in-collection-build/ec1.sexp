(case "ec1 Map.of-style bulk build where VALUES are perform results (list literal of performs)"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Cnt n
                ((bump (u) s (resume s (+ s 1))))
                (do
                  (def m (Map.insert (Map.insert Map.empty 1 (Cnt.bump)) 2 (Cnt.bump)))
                  (+ (* 10 (match (Map.lookup m 1) ((Some v) v) (_ -1)))
                     (match (Map.lookup m 2) ((Some v) v) (_ -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
