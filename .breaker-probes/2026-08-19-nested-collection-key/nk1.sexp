(case "nk1 a trie of 30 SET-valued keys resolves nested-collection key descent"
  (input  (do
            (def (fill (: i Int64) (: m (Map (Set Int64) Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (Set.of (list i (+ i 100))) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (Set.of (list 115 15))) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 315 Int64)))
