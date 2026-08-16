(case "mp2 the nested-map bump at 30 outer keys x 3 inner each (multimap at scale)"
  (input  (do
            (def (bump (: outer (Map Int64 (Map Int64 Int64))) (: ok Int64) (: ik Int64))
              (match (Map.lookup outer ok)
                ((Some inner) (Map.insert outer ok (Map.insert inner ik 1)))
                ((None _u) (Map.insert outer ok (Map.insert Map.empty ik 1)))))
            (def (fill (: i Int64) (: m (Map Int64 (Map Int64 Int64))))
              (if (= i 0) m
                  (fill (- i 1) (bump m (% i 30) (/ i 30)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m 5) ((Some i) (Map.len i)) ((None _u) -1)))))
            (export main)))
  (call   main (: 90 Int64)) (output (: 303 Int64)))
