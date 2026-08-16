(case "gws1 GROWING-STATE shared-let — the push arm let-binds a value derived from the argument the CURRENT length and the seed bias, answers the binder packed with the length, and threads the LIST GROWN by the binder; the total draws all three stored values back out, so a binder aliased against the post-push list would corrupt both the rows and the total"
  (input  (do
            (effect G
              (op push (-> Int64 Int64))
              (op total (-> Int64)))
            (def (main (: n Int64))
              (handle G (list)
                ((push (x) st
                  (let ((v2 (+ x (+ (* (List.len st) 10) (% n 3)))))
                    (resume (+ (* v2 10) (% (List.len st) 10))
                            (List.push st v2))))
                 (total () st
                  (match (List.at st 0)
                    ((Some p)
                      (match (List.at st 1)
                        ((Some q)
                          (match (List.at st 2)
                            ((Some r) (resume (+ p (+ q r)) st))
                            ((None) (resume (: -1 Int64) st))))
                        ((None) (resume (: -1 Int64) st))))
                    ((None) (resume (: -1 Int64) st)))))
                (let ((a (G.push (: 3 Int64))))
                  (let ((b (G.push (: 5 Int64))))
                    (let ((c (G.push (: 2 Int64))))
                      (let ((f (G.total)))
                        (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 40161232043 Int64))
  (call   main (: 0 Int64)) (output (: 30151222040 Int64)))
