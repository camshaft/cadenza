(case "cm3d SCOPE: THREE dispatches — the frozen payload is ALWAYS dispatch 1's (correct 60 = 15+22+23; stale runs 90 = 15+30+45, k=15 every time)"
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
                   (+ (M.step (Cmd.Go 7))
                      (M.step (Cmd.Go 1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 60 Int64)))
