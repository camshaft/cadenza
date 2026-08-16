(case "fz1 a fold over Map.to-list REBUILDS a filtered map at depth (the select-rebuild idiom)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 2)))))
            (def (keep-even (: ps (List (Tuple Int64 Int64))) (: m (Map Int64 Int64)))
              (match ps
                ((list) m)
                ((list h .. t) (match h ((tuple k v)
                  (keep-even t (if (= (% k 2) 0) (Map.insert m k v) m)))))))
            (def (main (: n Int64))
              (do
                (def src (fill n Map.empty))
                (def evens (keep-even (Map.to-list src) Map.empty))
                (+ (* 10 (Map.len evens))
                   (match (Map.lookup evens 30) ((Some v) (if (= v 60) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 60 Int64)) (output (: 301 Int64)))
