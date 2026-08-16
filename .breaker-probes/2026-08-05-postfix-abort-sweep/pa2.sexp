(case "pa2 NESTED aborts BOTH reading heap states: inner aborts to outer body, outer aborts to program"
  (input  (do
            (effect A (op ahalt (-> Unit Int64)))
            (effect B (op bhalt (-> Unit Int64)))
            (def (main (: a Int64))
              (+ 1 (handle A (list a)
                     ((ahalt (u) s (* 1000 (List.len s))))
                     (+ 10 (handle B (list a (+ a 1))
                             ((bhalt (u) t (* 100 (List.len t))))
                             (+ (B.bhalt) (A.ahalt)))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 211 Int64)))
