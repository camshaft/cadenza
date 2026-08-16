(case "cmopt SCOPE: the outer match is on a STD Option payload — is the stale binder user-sum-specific"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (effect M (op step (-> (Option Int64) Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match c
                    ((Some k) (match s
                                ((Mode.Idle) (resume k (Mode.Run k)))
                                ((Mode.Run j) (resume (+ j k) (Mode.Run (+ j k))))))
                    ((None) (resume -5 s)))))
                (+ (M.step (Some (+ 10 n)))
                   (M.step (Some 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 37 Int64))
  (call   main (: 0 Int64)) (output (: 27 Int64)))
