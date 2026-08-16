(case "mv2 updating ONE deep value leaves the other 39 heap values live (path-copy value isolation)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 (List Int64))))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (list i (* i 2))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def m2 (Map.insert m 25 (list 999)))
                (+ (* 100 (match (Map.lookup m2 25) ((Some xs) (List.len xs)) ((None _u) -1)))
                   (+ (* 10 (match (Map.lookup m2 24) ((Some xs) (List.len xs)) ((None _u) -1)))
                      (match (Map.lookup m 25) ((Some xs) (List.len xs)) ((None _u) -1))))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 122 Int64)))
