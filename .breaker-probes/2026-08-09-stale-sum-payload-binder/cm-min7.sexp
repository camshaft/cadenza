(case "cmmin7 same shape but inner match on a SCALAR state via literal patterns"
  (input  (do
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M 0
                ((step (c) s
                  (match c
                    ((Cmd.Go k) (match s
                                  (0 (resume k 1))
                                  (_ (resume (+ s k) s)))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 23 Int64)))
