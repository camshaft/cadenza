(case "hi4 the arm-installed handle ABORTS with an outer draw — the av face nested inside another handler's dispatch"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect M (op grab (-> Int64)))
            (effect Bail (op out (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle M 0
                  ((grab () m (resume (handle Bail 0
                                        ((out () t (O.next)))
                                        (+ (Bail.out) 999))
                                      m)))
                  (+ (* 10 (M.grab)) (O.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -4 Int64)) (output (: -43 Int64)))
