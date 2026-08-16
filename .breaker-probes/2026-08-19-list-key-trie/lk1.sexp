(case "lk1 a trie of 40 LIST keys (varied length) resolves element-wise content descent"
  (input  (do
            (def (mk (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc (mk (- i 1) (List.push acc i))))
            (def (fill (: i Int64) (: m (Map (List Int64) Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m (mk i (list)) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (mk 25 (list))) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 425 Int64)))
