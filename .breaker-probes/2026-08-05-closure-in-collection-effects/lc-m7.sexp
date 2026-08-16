(case "m7 LIST.at with perform key + perform-fed application (same shape, List not Map)"
  (input  (do
            (effect St (op feed (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def ops (list (fn ((: x Int64)) (* x 2)) (fn ((: x Int64)) (+ x 1000))))
                (handle St n
                  ((feed (u) s (resume s (+ s 1))))
                  (match (List.at ops (% (St.feed) 2))
                    ((Some f) (f (St.feed)))
                    ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1006 Int64)))
