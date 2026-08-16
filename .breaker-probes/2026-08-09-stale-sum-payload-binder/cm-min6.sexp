(case "cmmin6 payload binder crosses into an inner IF (not match) with two resume sites"
  (input  (do
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M 0
                ((step (c) s
                  (match c
                    ((Cmd.Go k) (if (= s 0)
                                    (resume k 1)
                                    (resume (+ s k) s))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 23 Int64)))
