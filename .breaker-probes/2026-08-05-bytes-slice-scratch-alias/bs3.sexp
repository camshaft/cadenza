(case "bs3 CONTROL: DIRECT Bytes (no lookup) + perform-threaded start/len"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def bs (Bytes.of (list 10 20 30 40 50 60 70 80)))
                (handle St n
                  ((next (u) s (resume s (+ s 1))))
                  (match (Bytes.slice bs (St.next) (St.next))
                    ((Some sl) (+ (* 10 (Bytes.len sl))
                                  (match (Bytes.at sl 0)
                                    ((Some b) (Int64.of b))
                                    ((None _u) -1))))
                    ((None _u) -100)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 40 Int64)))
