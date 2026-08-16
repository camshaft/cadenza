(case "m4 map-of-closures under a handler, CONSTANT key (no perform in the lookup)"
  (input  (do
            (effect St (op feed (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def ops (Map.insert (Map.insert Map.empty 0 (fn ((: x Int64)) (* x 2))) 1 (fn ((: x Int64)) (+ x 1000))))
                (handle St n
                  ((feed (u) s (resume s (+ s 1))))
                  (match (Map.lookup ops 1)
                    ((Some f) (f (St.feed)))
                    ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1005 Int64)))
