(case "mc2 a multi-shot arm SUMS re-reductions that each push onto a heap List"
  (input  (do
            (effect Go (op fork (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Go 0
                ((fork (u) s (+ (resume 1 s) (resume 2 s))))
                (List.len (List.push (List.push (list n) (Go.fork)) 7))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))
