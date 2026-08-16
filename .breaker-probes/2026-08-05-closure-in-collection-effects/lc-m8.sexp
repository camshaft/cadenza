(case "m8 scalar Map twin: values are Int64 not closures, same double-perform shape"
  (input  (do
            (effect St (op feed (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def ops (Map.insert (Map.insert Map.empty 0 20) 1 1000))
                (handle St n
                  ((feed (u) s (resume s (+ s 1))))
                  (match (Map.lookup ops (% (St.feed) 2))
                    ((Some v) (+ v (St.feed)))
                    ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1006 Int64)))
