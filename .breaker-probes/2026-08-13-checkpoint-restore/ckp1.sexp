(case "ckp1 a CHECKPOINT/RESTORE protocol — save copies the live slot into the shadow slot, work mutates only the live one, restore copies the shadow back and later work resumes from the checkpoint"
  (input  (do
            (effect S
              (op work (-> Int64 Int64))
              (op save (-> Int64))
              (op restore (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple n 0)
                ((work (v) st
                  (match st
                    ((tuple live sv)
                      (let ((l2 (+ (* live 2) v)))
                        (resume l2 (tuple l2 sv))))))
                 (save () st
                  (match st
                    ((tuple live _sv) (resume live (tuple live live)))))
                 (restore () st
                  (match st
                    ((tuple _live sv) (resume sv (tuple sv sv))))))
                (let ((a (S.work 1)))
                  (let ((b (S.save)))
                    (let ((c (S.work 5)))
                      (let ((d (S.work 2)))
                        (let ((e (S.restore)))
                          (let ((f (S.work 0)))
                            (+ (* 100 (+ (* 100 (+ (* 1000 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 707190400714 Int64))
  (call   main (: 0 Int64)) (output (: 101070160102 Int64)))
