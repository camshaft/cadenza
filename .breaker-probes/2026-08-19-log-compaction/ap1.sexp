(case "ap1 an APPEND-ONLY log with periodic COMPACTION: list grows then folds into a snapshot trie"
  (input  (do
            (def (applog (: i Int64) (: log (List (Tuple Int64 Int64))))
              (if (= i 0) log (applog (- i 1) (List.push log (tuple (% i 6) i)))))
            (def (compact (: xs (List (Tuple Int64 Int64))) (: snap (Map Int64 Int64)))
              (match xs
                ((list) snap)
                ((list h .. t) (match h ((tuple k v)
                  (compact t (Map.insert snap k v)))))))
            (def (main (: n Int64))
              (do
                (def log (applog n (list)))
                (def snap (compact log Map.empty))
                (+ (* 100 (Map.len snap))
                   (match (Map.lookup snap 3) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 603 Int64)))
