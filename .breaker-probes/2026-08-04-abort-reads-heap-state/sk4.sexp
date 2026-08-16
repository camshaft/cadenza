(case "sk4 NESTED handles BOTH with heap seeds; inner abort reads inner state through the outer region"
  (input  (do
            (effect A (op ct (-> Unit Int64)))
            (effect B (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle A (list 9)
                ((ct (u) s (resume (List.len s) s)))
                (+ (handle B Map.empty
                     ((halt (u) t (* 100 (Map.len t))))
                     (B.halt))
                   (A.ct))))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 1 Int64)))
