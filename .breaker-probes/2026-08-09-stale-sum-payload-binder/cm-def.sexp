(case "cmdef SCOPE: the inner sum-state match lives in a HELPER DEF taking the payload as a parameter — does the freeze reach through a def boundary"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (decide (: s Mode) (: k Int64))
              (match s
                ((Mode.Idle) (tuple k (Mode.Run k)))
                ((Mode.Run j) (tuple (+ j k) (Mode.Run (+ j k))))))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match c
                    ((Cmd.Go k) (match (decide s k)
                                  ((tuple v s2) (resume v s2)))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 37 Int64))
  (call   main (: 0 Int64)) (output (: 27 Int64)))
