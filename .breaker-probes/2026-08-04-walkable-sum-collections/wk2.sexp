(case "wk2 map-eq over a list-free recursive-sum KEY converges after divergent overwrite histories"
  (input  (do
            (type T (TI Int64) (TP T T))
            (def (mk (: i Int64)) (T.TP (T.TI i) (T.TI (* 2 i))))
            (def (up (: i Int64) (: n Int64) (: m (Map T Int64)))
              (if (> i n) m (up (+ i 1) n (Map.insert m (mk i) i))))
            (def (noisy (: i Int64) (: n Int64) (: m (Map T Int64)))
              (if (> i n) m
                (noisy (+ i 1) n (Map.insert (Map.insert m (mk i) (- 0 i)) (mk i) i))))
            (def (main (: n Int64))
              (if (= (up 1 n Map.empty) (noisy 1 n Map.empty)) 1 0))
            (export main)))
  (call   main (: 60 Int64)) (output (: 1 Int64)))
