(case "ec3 a Set literal of perform results — dedup when two performs COLLIDE by arm design"
  (input  (do
            (effect Cnt (op same (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Cnt n
                ((same (u) s (resume s s)))
                (do
                  (def s2 (Set.insert (Set.insert (Set.of (list)) (Cnt.same)) (Cnt.same)))
                  (Set.len s2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
