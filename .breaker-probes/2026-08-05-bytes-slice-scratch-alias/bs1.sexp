(case "bs1 Bytes.slice of a Map-looked-up Bytes with perform-threaded start and len"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def table (Map.insert Map.empty 1 (Bytes.of (list 10 20 30 40 50 60 70 80))))
                (handle St n
                  ((next (u) s (resume s (+ s 1))))
                  (match (Map.lookup table 1)
                    ((Some bs)
                      (match (Bytes.slice bs (St.next) (St.next))
                        ((Some sl) (+ (* 10 (Bytes.len sl))
                                      (match (Bytes.at sl 0)
                                        ((Some b) (Int64.of b))
                                        ((None _u) -1))))
                        ((None _u) -100)))
                    ((None _u) -200)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 40 Int64)))
