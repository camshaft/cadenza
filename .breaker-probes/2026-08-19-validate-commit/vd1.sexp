(case "vd1 a two-phase VALIDATE-then-COMMIT walk: all-or-nothing over a staged trie"
  (input  (do
            (def (stage (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (stage (- i 1) (Map.insert m i (* i 4)))))
            (def (valid (: ps (List (Tuple Int64 Int64))))
              (match ps
                ((list) true)
                ((list h .. t) (match h ((tuple _k v) (if (> v 200) false (valid t)))))))
            (def (main (: n Int64))
              (do
                (def ok-batch (stage n Map.empty))
                (def bad-batch (Map.insert (stage n Map.empty) 99 500))
                (+ (* 10 (if (valid (Map.to-list ok-batch)) (Map.len ok-batch) -1))
                   (if (valid (Map.to-list bad-batch)) -1 1))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 401 Int64)))
