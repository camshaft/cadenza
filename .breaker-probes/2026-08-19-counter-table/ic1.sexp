(case "ic1 an INCREMENTAL COUNTER table: read-bump-write cycles interleaved with queries"
  (input  (do
            (def (bump (: m (Map Int64 Int64)) (: k Int64))
              (Map.insert m k (+ 1 (match (Map.lookup m k) ((Some c) c) ((None _u) 0)))))
            (def (feed (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (feed (- i 1) (bump m (% i 5)))))
            (def (main (: n Int64))
              (do
                (def m (feed n Map.empty))
                (+ (* 100 (match (Map.lookup m 0) ((Some c) c) ((None _u) -1)))
                   (+ (* 10 (match (Map.lookup m 3) ((Some c) c) ((None _u) -1)))
                      (Map.len m)))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 555 Int64)))
