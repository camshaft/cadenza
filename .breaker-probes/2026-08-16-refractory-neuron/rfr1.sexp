(case "rfr1 an INTEGRATE-AND-FIRE neuron with a refractory period — each excite adds the input plus a seed bias to the potential, a crossing of ten FIRES (answer packs the crossing potential with a 7 tag) and silences the next two excitations (their answers pack the fire count with a 55 tag, potential frozen at zero), and the seed bias shifts WHICH excitation crosses so the refractory window swallows different inputs"
  (input  (do
            (effect N
              (op excite (-> Int64 Int64))
              (op fires (-> Int64)))
            (def (main (: n Int64))
              (handle N (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((excite (x) st
                  (match st
                    ((tuple pot r f)
                      (if (> r 0)
                          (resume (+ (* f 100) 55) (tuple pot (- r 1) f))
                          (if (>= (+ pot (+ x (% n 3))) 10)
                              (resume (+ (* (+ pot (+ x (% n 3))) 10) 7)
                                      (tuple (: 0 Int64) (: 2 Int64) (+ f 1)))
                              (resume (+ pot (+ x (% n 3)))
                                      (tuple (+ pot (+ x (% n 3))) r f)))))))
                 (fires () st
                  (match st ((tuple pot r f) (resume f st)))))
                (let ((a (N.excite (: 4 Int64))))
                  (let ((b (N.excite (: 5 Int64))))
                    (let ((c (N.excite (: 3 Int64))))
                      (let ((d (N.excite (: 6 Int64))))
                        (let ((e (N.excite (: 8 Int64))))
                          (let ((g (N.fires)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) g)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5117155155009001 Int64))
  (call   main (: 0 Int64)) (output (: 4009127155155001 Int64)))
