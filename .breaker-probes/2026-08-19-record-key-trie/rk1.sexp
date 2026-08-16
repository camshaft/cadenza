(case "rk1 a trie of 40 RECORD keys resolves descriptor-ordered field descent at depth"
  (input  (do
            (def (fill (: i Int64) (: m (Map (Record (x Int64) (y Int64)) Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m (record (x (% i 6)) (y (/ i 6))) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (record (x 4) (y 5))) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 434 Int64)))
