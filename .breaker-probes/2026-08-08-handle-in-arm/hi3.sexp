(case "hi3 the arm-installed handle's OWN arm resumes with an OUTER draw — the rv face nested inside another handler's dispatch"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect M (op grab (-> Int64)))
            (effect J (op ask (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle M 0
                  ((grab () m (resume (handle J 0
                                        ((ask () t (resume (O.next) t)))
                                        (+ (J.ask) (* 10 (J.ask))))
                                      m)))
                  (+ (M.grab) (* 100 (O.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 765 Int64))
  (call   main (: 0 Int64)) (output (: 210 Int64))
  (call   main (: -1 Int64)) (output (: 99 Int64)))
