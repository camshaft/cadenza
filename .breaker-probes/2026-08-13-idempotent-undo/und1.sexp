(case "und1 an IDEMPOTENT-UNDO counter — the tuple state carries (value,last-delta); undo reverses the last delta exactly once and answers it, the second undo answers zero and no-ops"
  (input  (do
            (effect S
              (op apply (-> Int64 Int64))
              (op undo (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple n 0)
                ((apply (d) st
                  (match st
                    ((tuple v _l) (resume (+ v d) (tuple (+ v d) d)))))
                 (undo () st
                  (match st
                    ((tuple v l)
                      (if (= l 0)
                          (resume 0 st)
                          (resume l (tuple (- v l) 0)))))))
                (let ((a (S.apply 5)))
                  (let ((b (S.apply -2)))
                    (let ((c (S.undo)))
                      (let ((d (S.undo)))
                        (let ((e (S.apply 7)))
                          (+ (* 100 (+ (* 10 (+ (* 100 (+ (* 100 a) (+ b 10))) (+ c 10))) (+ d 1))) e))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 81608115 Int64))
  (call   main (: 20 Int64)) (output (: 253308132 Int64)))
