(case "cm2ops SCOPE: the two dispatches go through DIFFERENT ops sharing the arm shape — per-op or per-arm staleness"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)) (op step2 (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match c
                    ((Cmd.Go k) (match s
                                  ((Mode.Idle) (resume k (Mode.Run k)))
                                  ((Mode.Run j) (resume (+ j k) (Mode.Run (+ j k))))))))
                 (step2 (c) s
                  (match c
                    ((Cmd.Go k) (match s
                                  ((Mode.Idle) (resume k (Mode.Run k)))
                                  ((Mode.Run j) (resume (+ j k) (Mode.Run (+ j k)))))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step2 (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 37 Int64))
  (call   main (: 0 Int64)) (output (: 27 Int64)))
