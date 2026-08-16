(case "tkmin5 min3 WITHOUT qry — rec + cnt only (unused-map binder in cnt)"
  (input  (do
            (effect E (op rec (-> Int64)) (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n Map.empty)
                ((rec () st (match st
                              ((tuple s m)
                               (resume s (tuple (+ s 2)
                                                (Map.insert m (tuple s (+ s 1)) (* 10 s)))))))
                 (cnt () st (match st ((tuple s m) (resume s st)))))
                (do (E.rec) (E.cnt))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7 Int64)))
