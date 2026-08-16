(case "cmmin5 nested match arg-outer/state-inner but the SECOND payload differs only in ONE branch"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match c
                    ((Cmd.Go k) (match s
                                  ((Mode.Idle) (resume k (Mode.Run k)))
                                  ((Mode.Run j) (resume (+ j k) (Mode.Run (+ j k)))))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 37 Int64)))
