(case "sw1 Set.contains on a Map-looked-up Set with a perform-threaded element"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def table (Map.insert Map.empty 1 (Set.of (list 2 5 9))))
                (handle St n
                  ((next (u) s (resume s (+ s 1))))
                  (match (Map.lookup table 1)
                    ((Some st)
                      (+ (if (Set.contains st (St.next)) 10 0)
                         (if (Set.contains st (St.next)) 1 0)))
                    ((None _u) -200)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64)))
