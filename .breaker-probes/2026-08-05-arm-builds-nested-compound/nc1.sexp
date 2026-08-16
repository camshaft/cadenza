(case "nc1 the arm builds a NESTED compound (Map inside a tuple) as the resume value, body probes both levels"
  (input  (do
            (effect St (op snap (-> Unit (Tuple Int64 (Map Int64 Int64)))))
            (def (main (: n Int64))
              (handle St n
                ((snap (u) s (resume (tuple s (Map.insert Map.empty s (* s 10))) (+ s 1))))
                (match (St.snap)
                  ((tuple v m)
                    (+ (* 100 v)
                       (match (Map.lookup m v) ((Some x) x) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 550 Int64)))
