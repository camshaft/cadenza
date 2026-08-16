(case "wp4b control: same mid-sequence abort WITHOUT the recursive walk (flat 3-perform sequence)"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)) (op halt (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Cnt 0
                ((bump (u) s (resume s (+ s 1)))
                 (halt (u) s (* 1000 s)))
                (+ (Cnt.bump) (+ (Cnt.bump) (+ (Cnt.halt) (Cnt.bump))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 2000 Int64)))
