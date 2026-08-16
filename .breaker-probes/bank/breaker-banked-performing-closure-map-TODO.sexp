(case "a performing closure dispatched through a MAP lookup threads the handler state"
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (main (: n Int64) (: k Int64))
              (handle Ctr n
                ((next (u) s (resume s (+ s 1))))
                (let ((m (Map.insert (Map.insert Map.empty
                            1 (fn ((: u Unit)) (Ctr.next unit)))
                            2 (fn ((: u Unit)) (* (Ctr.next unit) 100)))))
                  (+ (* 10 (match (Map.lookup m k) ((Some f) (f unit)) ((None u2) -1)))
                     (match (Map.lookup m 1) ((Some g) (g unit)) ((None u3) -1))))))
            (export main)))
  (call   main (: 3 Int64) (: 1 Int64)) (output (: 34 Int64))
  (call   main (: 3 Int64) (: 2 Int64)) (output (: 3004 Int64)))
