(case "cmgrn the STATE-OUTER nesting order threads fresh payloads correctly across three dispatches — the working twin of the arg-outer freeze"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match s
                    ((Mode.Idle) (match c ((Cmd.Go k) (resume k (Mode.Run k)))))
                    ((Mode.Run j) (match c ((Cmd.Go k) (resume (+ j k) (Mode.Run (+ j k)))))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (+ (M.step (Cmd.Go 7))
                      (M.step (Cmd.Go 1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 60 Int64))
  (call   main (: 0 Int64)) (output (: 45 Int64))
  (call   main (: -3 Int64)) (output (: 36 Int64)))
