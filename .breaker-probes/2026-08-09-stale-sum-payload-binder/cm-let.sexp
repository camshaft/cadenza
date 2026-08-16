(case "cmlet SCOPE: a LET-derived value (m = 2k) inside the outer branch, read by the inner sum-state match — is the freeze binder-only or whole-environment"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match c
                    ((Cmd.Go k)
                     (let ((m (* 2 k)))
                       (match s
                         ((Mode.Idle) (resume m (Mode.Run m)))
                         ((Mode.Run j) (resume (+ j m) (Mode.Run (+ j m))))))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 74 Int64))
  (call   main (: 0 Int64)) (output (: 54 Int64)))
