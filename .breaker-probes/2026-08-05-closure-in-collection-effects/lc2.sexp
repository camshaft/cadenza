(case "lc2 a MAP of named strategies — the perform result picks WHICH closure runs"
  (input  (do
            (effect St (op pick (-> Unit Int64)) (op feed (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def ops (Map.insert (Map.insert Map.empty 0 (fn ((: x Int64)) (* x 2))) 1 (fn ((: x Int64)) (+ x 1000))))
                (handle St n
                  ((pick (u) s (resume (% s 2) (+ s 1)))
                   (feed (u) s (resume s (+ s 1))))
                  (match (Map.lookup ops (St.pick))
                    ((Some f) (f (St.feed)))
                    ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1006 Int64)))
