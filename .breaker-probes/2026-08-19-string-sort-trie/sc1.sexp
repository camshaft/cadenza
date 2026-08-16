(case "sc1 String values at trie depth SORT via an insertion-sort walk (comparator over retrievals)"
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "z") (- n 1))))
            (def (fill (: i Int64) (: m (Map Int64 String)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (rep "k" (% i 4))))))
            (def (insort (: x String) (: xs (List String)))
              (match xs
                ((list) (list x))
                ((list h .. t) (if (< x h) (List.concat (list x) xs) (List.concat (list h) (insort x t))))))
            (def (walk (: ps (List (Tuple Int64 String))) (: acc (List String)))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple _k v) (walk t (insort v acc)))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def sorted (walk (Map.to-list m) (list)))
                (+ (* 10 (List.len sorted))
                   (match (List.at sorted 0) ((Some s) (String.byte-len s)) ((None _u) -1)))))
            (export main)))
  (call   main (: 12 Int64)) (output (: 121 Int64)))
