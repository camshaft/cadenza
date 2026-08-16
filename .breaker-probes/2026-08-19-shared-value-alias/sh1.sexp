(case "sh1 one inner map shared under TWO outer keys updates independently (no aliasing)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 10)))))
            (def (main (: n Int64))
              (do
                (def inner (fill n Map.empty))
                (def outer (Map.insert (Map.insert Map.empty 1 inner) 2 inner))
                (def bumped (match (Map.lookup outer 1)
                              ((Some m1) (Map.insert outer 1 (Map.insert m1 999 1)))
                              ((None _u) outer)))
                (+ (* 100 (match (Map.lookup bumped 1) ((Some m1) (Map.len m1)) ((None _u) -1)))
                   (match (Map.lookup bumped 2) ((Some m2) (Map.len m2)) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 4140 Int64)))
