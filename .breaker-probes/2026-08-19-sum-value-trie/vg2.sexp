(case "vg2 a fold over Map.to-list dispatches EVERY variant of a 40-entry sum-valued trie"
  (input  (do
            (type Ev (Add Int64) (Del Int64) (Noop))
            (def (fill (: i Int64) (: m (Map Int64 Ev)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m i
                  (if (= (% i 3) 0) (Ev.Noop)
                    (if (= (% i 3) 1) (Ev.Add i) (Ev.Del i)))))))
            (def (net (: ps (List (Tuple Int64 Ev))) (: acc Int64))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple _k e)
                  (net t (+ acc (match e ((Ev.Add v) v) ((Ev.Del v) (- 0 v)) ((Ev.Noop) 0)))))))))
            (def (main (: n Int64))
              (net (Map.to-list (fill n Map.empty)) 0))
            (export main)))
  (call   main (: 40 Int64)) (output (: 27 Int64)))
