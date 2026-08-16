(case "ss6 the slot SELECTOR is derived from the state itself — |a mod 3| routes each bump, a same-slot revisit and a two-slot walk both pinned"
  (input  (do
            (effect E (op step (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 10 100)
                ((step () st (match st
                               ((tuple a b c)
                                (let ((m (% a 3)))
                                  (let ((i (if (< m 0) (- 0 m) m)))
                                    (if (= i 0) (resume a (tuple (+ a 1) b c))
                                        (if (= i 1) (resume b (tuple a (+ b 1) c))
                                            (resume c (tuple a b (+ c 1)))))))))))
                (+ (E.step) (E.step))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 201 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64))
  (call   main (: -4 Int64)) (output (: 21 Int64)))
