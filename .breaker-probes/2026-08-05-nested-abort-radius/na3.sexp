(case "na3 control: an abort arm performing a DIFFERENT OUTER handler's op"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op halt (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s s)))
                (handle B 0
                  ((halt (u) t (* 100 (A.get))))
                  (+ 5 (B.halt)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 700 Int64)))
