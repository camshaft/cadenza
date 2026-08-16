(case "cmloc SCOPE: the inner match scrutinizes a LOCALLY-BUILT Option from k (not the state) — is state-scrutiny required for the freeze"
  (input  (do
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M 0
                ((step (c) s
                  (match c
                    ((Cmd.Go k)
                     (match (if (> k 8) (Some k) (None))
                       ((Some x) (resume (+ x s) (+ s 1)))
                       ((None) (resume 0 s)))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 15 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64)))
