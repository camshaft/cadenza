(case "cmmin2 sum ARG only, SCALAR state - two consecutive Go dispatches with different payloads"
  (input  (do
            (type Cmd (Go Int64) (Halt))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M 0
                ((step (c) s
                  (match c
                    ((Cmd.Go k) (resume (+ k s) (+ s 100)))
                    ((Cmd.Halt) (resume -5 s)))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 122 Int64)))
