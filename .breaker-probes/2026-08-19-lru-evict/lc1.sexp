(case "lc1 an LRU-flavored eviction: capacity-bounded trie with access-order tracking"
  (input  (do
            (def (touch (: m (Map Int64 Int64)) (: order (List Int64)) (: k Int64) (: clock Int64))
              (tuple (Map.insert m k clock) (List.push order k)))
            (def (evict-oldest (: m (Map Int64 Int64)) (: ps (List (Tuple Int64 Int64))) (: bk Int64) (: bt Int64))
              (match ps
                ((list) (Map.remove m bk))
                ((list h .. t) (match h ((tuple k ts)
                  (if (< ts bt) (evict-oldest m t k ts) (evict-oldest m t bk bt)))))))
            (def (feed (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (> i n) m
                (let ((m2 (Map.insert m (% i 7) i)))
                  (feed (+ i 1) n
                    (if (> (Map.len m2) 4) (evict-oldest m2 (Map.to-list m2) -1 99999) m2)))))
            (def (main (: n Int64))
              (do
                (def m (feed 1 n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (% n 7)) ((Some _v) 1) ((None _u) 0)))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 41 Int64)))
