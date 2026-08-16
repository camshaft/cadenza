(case "m5 map-of-closures, perform-computed key, but f applied to a CONSTANT (single op)"
  (input  (do
            (effect St (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def ops (Map.insert (Map.insert Map.empty 0 (fn ((: x Int64)) (* x 2))) 1 (fn ((: x Int64)) (+ x 1000))))
                (handle St n
                  ((pick (u) s (resume (% s 2) (+ s 1))))
                  (match (Map.lookup ops (St.pick))
                    ((Some f) (f 6))
                    ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1006 Int64)))
