(case "sh3 a shared inner map reached through BOTH outer keys stays one canonical value for keying"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
            (def (main (: n Int64))
              (do
                (def inner (fill n Map.empty))
                (def outer (Map.insert (Map.insert Map.empty 1 inner) 2 inner))
                (def k1 (match (Map.lookup outer 1) ((Some m1) m1) ((None _u) Map.empty)))
                (def k2 (match (Map.lookup outer 2) ((Some m2) m2) ((None _u) Map.empty)))
                (match (Map.lookup (Map.insert Map.empty k1 42) k2)
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 42 Int64)))
