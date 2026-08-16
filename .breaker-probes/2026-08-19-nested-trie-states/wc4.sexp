(case "wc4 the cross-level face via LET: inner body performs the OUTER effect too"
  (input  (do
            (effect Ou (op oput (-> Int64 Int64)) (op olen (-> Unit Int64)))
            (effect In (op iput (-> Int64 Int64)) (op ilen (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Ou Map.empty
                ((oput (v) s (resume 0 (Map.insert s v v)))
                 (olen (u) s (resume (Map.len s) s)))
                (do
                  (Ou.oput 1)
                  (let ((inner-r
                          (handle In Map.empty
                            ((iput (v) t (resume 0 (Map.insert t v v)))
                             (ilen (u) t (resume (Map.len t) t)))
                            (do
                              (In.iput 10)
                              (In.iput 20)
                              (Ou.oput 2)
                              (In.ilen)))))
                    (+ (* 100 inner-r) (Ou.olen))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 202 Int64)))
