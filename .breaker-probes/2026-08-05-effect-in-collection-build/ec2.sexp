(case "ec2 perform results as Map KEYS (each key a distinct state read)"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Cnt n
                ((bump (u) s (resume s (+ s 1))))
                (do
                  (def m (Map.insert (Map.insert Map.empty (Cnt.bump) 100) (Cnt.bump) 200))
                  (+ (* 10 (Map.len m))
                     (match (Map.lookup m (+ n 1)) ((Some v) (/ v 100)) (_ -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 22 Int64)))
