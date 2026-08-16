(case "cm2arg SCOPE: TWO sum-typed op args matched in sequence before the state match — do both binders freeze"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64))
            (effect M (op pair (-> Cmd Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((pair (c1 c2) s
                  (match c1
                    ((Cmd.Go k1)
                     (match c2
                       ((Cmd.Go k2)
                        (match s
                          ((Mode.Idle) (resume (+ k1 k2) (Mode.Run (+ k1 k2))))
                          ((Mode.Run j) (resume (+ j (+ k1 k2)) (Mode.Run (+ j (+ k1 k2))))))))))))
                (+ (M.pair (Cmd.Go (+ 10 n)) (Cmd.Go 3))
                   (M.pair (Cmd.Go 1) (Cmd.Go 2)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 39 Int64))
  (call   main (: 0 Int64)) (output (: 29 Int64)))
