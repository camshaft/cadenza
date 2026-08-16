(case "tk-ann1 CONTROL: the seed Map ANNOTATED with its tuple key type — does an explicit ascription close the cross-arm gap"
  (input  (do
            (effect E (op rec (-> Int64)) (op qry (-> Int64 Int64 Int64)) (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (: Map.empty (Map (Tuple Int64 Int64) Int64)))
                ((rec () st (match st
                              ((tuple s m)
                               (resume s (tuple (+ s 2)
                                                (Map.insert m (tuple s (+ s 1)) (* 10 s)))))))
                 (qry (a b) st (match st
                                 ((tuple s m)
                                  (resume (match (Map.lookup m (tuple a b))
                                            ((Some v) v)
                                            ((None) -1))
                                          st))))
                 (cnt () st (match st ((tuple s m) (resume s st)))))
                (do (E.rec) (+ (E.qry n (+ n 1)) (E.cnt)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 57 Int64)))
