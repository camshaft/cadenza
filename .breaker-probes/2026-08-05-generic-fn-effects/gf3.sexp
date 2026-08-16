(case "gf3 a generic RECURSIVE fold over a list of perform results (build then fold under one handle)"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)))
            (def (suml xs)
              (match xs
                ((list) 0)
                ((list h .. t) (+ h (suml t)))))
            (def (grab (: k Int64) (: acc (List Int64)))
              (if (= k 0) acc (grab (- k 1) (List.push acc (Cnt.bump)))))
            (def (main (: n Int64))
              (handle Cnt n
                ((bump (u) s (resume s (+ s 1))))
                (suml (grab 4 (list)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 26 Int64)))
