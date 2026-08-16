(case "qq6 an evaluated quasiquote builds a HEAP value the surrounding code consumes"
  (input  (do
            (def (main (: k Int64))
              (match (eval (quasiquote (list (unquote k) (* (unquote k) 2))))
                (xs (+ (List.len xs)
                       (* 10 (match (List.at xs 1) ((Some v) v) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 102 Int64)))
