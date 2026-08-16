(case "tkmin2 tuple-keyed Map as HANDLER STATE, one insert one lookup"
  (input  (do
            (effect E (op rec (-> Int64)) (op qry (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple n Map.empty)
                ((rec () st (match st
                              ((tuple s m)
                               (resume s (tuple (+ s 2)
                                                (Map.insert m (tuple s (+ s 1)) (* 10 s)))))))
                 (qry (a b) st (match st
                                 ((tuple s m)
                                  (resume (match (Map.lookup m (tuple a b))
                                            ((Some v) v)
                                            ((None) -1))
                                          st)))))
                (do (E.rec) (E.qry n (+ n 1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64)))
