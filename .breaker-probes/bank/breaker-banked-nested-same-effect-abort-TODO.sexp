(case "an abort under LEXICALLY NESTED same-effect handlers exits only the INNER region"
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: k Int64))
              (+ (handle Bail 0 ((bail (v) s (* v 1000)))
                   (+ 100 (handle Bail 0 ((bail (v2) s2 (* v2 10)))
                            (if (> k 5) (Bail.bail 7) 3))))
                 1))
            (export main)))
  (call   main (: 9 Int64)) (output (: 171 Int64))
  (call   main (: 1 Int64)) (output (: 104 Int64)))
