(case "tv1 TWIN: tuple VALUES (scalar keys) with a third arm — is the gap key-position-specific"
  (input  (do
            (effect E (op rec (-> Int64)) (op qry (-> Int64 Int64)) (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n Map.empty)
                ((rec () st (match st
                              ((tuple s m)
                               (resume s (tuple (+ s 2)
                                                (Map.insert m s (tuple s (* 10 s))))))))
                 (qry (k) st (match st
                               ((tuple s m)
                                (resume (match (Map.lookup m k)
                                          ((Some p) (match p ((tuple a b) (+ a b))))
                                          ((None) -1))
                                        st))))
                 (cnt () st (match st ((tuple s m) (resume s st)))))
                (do (E.rec) (+ (E.qry n) (E.cnt)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 62 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64)))
