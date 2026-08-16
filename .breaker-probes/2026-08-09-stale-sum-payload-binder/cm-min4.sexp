(case "cmmin4 nesting order flipped: match STATE outer, ARG inner"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64) (Halt))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match s
                    ((Mode.Idle) (match c
                                   ((Cmd.Go k) (resume k (Mode.Run k)))
                                   ((Cmd.Halt) (resume -5 (Mode.Idle)))))
                    ((Mode.Run j) (match c
                                    ((Cmd.Go k) (resume (+ j k) (Mode.Run (+ j k))))
                                    ((Cmd.Halt) (resume -5 (Mode.Idle))))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 37 Int64)))
