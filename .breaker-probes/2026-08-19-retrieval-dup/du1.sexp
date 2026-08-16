(case "du1 a trie value read TWICE in one expression (dup discipline through the descent)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 (List Int64))))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (list i (* i 2) (* i 3))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (match (Map.lookup m 20) ((Some xs) (List.len xs)) ((None _u) -1))
                   (match (Map.lookup m 20) ((Some xs) (match (List.at xs 2) ((Some v) v) ((None _u) -2))) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 63 Int64)))
