(case "nm1 a Map-of-Maps built and probed at both levels (nested CHAMP descent)"
  (input  (do
            (def (main (: n Int64))
              (do
                (def inner1 (Map.insert (Map.insert Map.empty 1 n) 2 (+ n 1)))
                (def inner2 (Map.insert Map.empty 9 99))
                (def outer (Map.insert (Map.insert Map.empty 10 inner1) 20 inner2))
                (+ (* 100 (match (Map.lookup outer 10)
                            ((Some m) (match (Map.lookup m 2) ((Some v) v) ((None _u) -1)))
                            ((None _u) -2)))
                   (match (Map.lookup outer 20)
                     ((Some m) (Map.len m))
                     ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 601 Int64)))
