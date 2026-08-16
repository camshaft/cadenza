(do
            (effect P (op pull (-> Int64)))
            (def (drive (: acc Int64))
              (match (P.pull)
                (v (if (= v (: -1 Int64))
                       acc
                       (drive (+ (* acc 100) (% v 100)))))))
            (def (main (: n Int64))
              (handle P (: 7 Int64)
                ((pull () cur
                  (if (> (+ cur (+ 6 (% n 3))) 40)
                      (resume (: -1 Int64) cur)
                      (resume (+ cur (+ 6 (% n 3))) (+ cur (+ 6 (% n 3)))))))
                (drive (: 0 Int64))))
            (export main))