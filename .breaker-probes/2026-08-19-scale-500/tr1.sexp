(case "tr1 a TAIL-recursive fold consuming Map.to-list at 200 entries runs without stack growth"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i 1))))
            (def (count (: ps (List (Tuple Int64 Int64))) (: acc Int64))
              (match ps
                ((list) acc)
                ((list _h .. t) (count t (+ acc 1)))))
            (def (main (: n Int64))
              (count (Map.to-list (fill n Map.empty)) 0))
            (export main)))
  (call   main (: 200 Int64)) (output (: 200 Int64)))
