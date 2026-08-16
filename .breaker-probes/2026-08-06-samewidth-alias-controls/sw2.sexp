(case "sw2 Map.lookup on a looked-up Map — key is perform-threaded (single-level indirection)"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def inner (Map.insert (Map.insert Map.empty 5 100) 6 250))
                (handle St n
                  ((next (u) s (resume s (+ s 1))))
                  (+ (match (Map.lookup inner (St.next))
                       ((Some v) v)
                       ((None _u) -1))
                     (match (Map.lookup inner (St.next))
                       ((Some v) v)
                       ((None _u) -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 350 Int64)))
