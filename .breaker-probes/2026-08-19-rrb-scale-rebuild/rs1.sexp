(case "rs1 a 300-element head-walk push rebuild equals its source (element-order identity at scale)"
  (input  (do
            (def (build (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc (build (- i 1) (List.push acc i))))
            (def (walk (: xs (List Int64)) (: acc (List Int64)))
              (match xs
                ((list) acc)
                ((list h .. t) (walk t (List.push acc h)))))
            (def (main (: n Int64))
              (do
                (def xs (build n (list)))
                (def rx (walk xs (list)))
                (+ (* 10 (if (= rx xs) 1 0))
                   (if (= (List.len rx) n) 1 0))))
            (export main)))
  (call   main (: 300 Int64)) (output (: 11 Int64)))
