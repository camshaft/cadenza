(case "cmtup SCOPE: the sum payload is a TUPLE — does the stale-binder hit destructured compound payloads"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go (Tuple Int64 Int64)))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match c
                    ((Cmd.Go p) (match s
                                  ((Mode.Idle) (match p ((tuple a b) (resume (+ a b) (Mode.Run (+ a b))))))
                                  ((Mode.Run j) (match p ((tuple a b) (resume (+ j (+ a b)) (Mode.Run j))))))))))
                (+ (M.step (Cmd.Go (tuple (+ 10 n) 2)))
                   (M.step (Cmd.Go (tuple 3 4))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 41 Int64))
  (call   main (: 0 Int64)) (output (: 31 Int64)))
