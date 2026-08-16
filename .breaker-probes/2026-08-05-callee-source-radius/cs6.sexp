(case "cs6 trap shape under HIGH live-local pressure (deep let chain around the lookup+apply)"
  (input  (do
            (effect St (op feed (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def ops (Map.insert (Map.insert Map.empty 0 (fn ((: x Int64)) (* x 2))) 1 (fn ((: x Int64)) (+ x 1000))))
                (handle St n
                  ((feed (u) s (resume s (+ s 1))))
                  (let ((a1 (+ n 1)))
                    (let ((a2 (* a1 2)))
                      (let ((a3 (- a2 3)))
                        (let ((a4 (+ a3 a1)))
                          (+ (match (Map.lookup ops (% (St.feed) 2))
                               ((Some f) (f (St.feed)))
                               ((None _u) -1))
                             (+ a1 (+ a2 (+ a3 a4)))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1048 Int64)))
