(case "du2 one retrieved heap value bound ONCE fans out to three consumers (borrow discipline)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 (List Int64))))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (list i (* i 2))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def xs (match (Map.lookup m 15) ((Some l) l) ((None _u) (list))))
                (+ (* 100 (List.len xs))
                   (+ (* 10 (match (List.at xs 0) ((Some v) v) ((None _u) -1)))
                      (match (List.at xs 1) ((Some v) (% v 10)) ((None _u) -1))))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 350 Int64)))
