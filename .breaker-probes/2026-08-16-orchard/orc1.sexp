(case "orc1 an ORCHARD of ripening trees in a MAP — tend inserts-or-bumps a tree's ripeness by two answering tree ripeness and grove size, pick REMOVES a tree only at the seed-shifted threshold (five or seven) counting it or refuses with a 900 tag leaving the map untouched, report packs picked grove-size and ripeness-sum low digit, and the same three tends ripen to six which one threshold accepts and the other refuses"
  (input  (do
            (effect R
              (op tend (-> Int64 Int64))
              (op pick (-> Int64 Int64))
              (op report (-> Int64)))
            (def (main (: n Int64))
              (handle R (tuple (: (Map.empty) (Map Int64 Int64)) (: 0 Int64))
                ((tend (t) st
                  (match st
                    ((tuple m picked)
                      (match (Map.lookup m t)
                        ((Some r)
                          (resume (+ (* t 100) (+ (* (+ r 2) 10) (Map.len (Map.insert m t (+ r 2)))))
                                  (tuple (Map.insert m t (+ r 2)) picked)))
                        ((None)
                          (resume (+ (* t 100) (+ (* 2 10) (Map.len (Map.insert m t 2))))
                                  (tuple (Map.insert m t 2) picked)))))))
                 (pick (t) st
                  (match st
                    ((tuple m picked)
                      (match (Map.lookup m t)
                        ((Some r)
                          (if (>= r (+ 5 (* (% n 3) 2)))
                              (resume (+ (: 500 Int64) r) (tuple (Map.remove m t) (+ picked 1)))
                              (resume (+ (: 900 Int64) r) st)))
                        ((None) (resume (: 999 Int64) st))))))
                 (report () st
                  (match st
                    ((tuple m picked)
                      (match (Map.lookup m 1)
                        ((Some r) (resume (+ (* picked 100) (+ (* (Map.len m) 10) (% r 10))) st))
                        ((None) (resume (+ (* picked 100) (* (Map.len m) 10)) st)))))))
                (let ((a (R.tend (: 1 Int64))))
                  (let ((b (R.tend (: 1 Int64))))
                    (let ((c (R.tend (: 1 Int64))))
                      (let ((d (R.pick (: 1 Int64))))
                        (let ((f (R.report)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 121141161906016 Int64))
  (call   main (: 0 Int64)) (output (: 121141161506100 Int64)))
