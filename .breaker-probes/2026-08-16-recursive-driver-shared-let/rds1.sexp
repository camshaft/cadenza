(case "rds1 RECURSIVE-DRIVER shared-let — the driver keeps pulling until the arm answers the stop sentinel, the arm let-binds the advanced value (current plus step plus seed bias) using it in the over-forty stop test the answer AND the threaded next-state, and the bias makes one run stop a hop EARLIER so the accumulated low-digit trails differ in length"
  (input  (do
            (effect P (op pull (-> Int64)))
            (def (drive (: acc Int64))
              (match (P.pull)
                (v (if (= v (: -1 Int64))
                       acc
                       (drive (+ (* acc 100) (% v 100)))))))
            (def (main (: n Int64))
              (handle P (: 7 Int64)
                ((pull () cur
                  (let ((v2 (+ cur (+ 6 (% n 3)))))
                    (if (> v2 40)
                        (resume (: -1 Int64) cur)
                        (resume v2 v2)))))
                (drive (: 0 Int64))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 14212835 Int64))
  (call   main (: 0 Int64)) (output (: 1319253137 Int64)))
