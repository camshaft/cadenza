(case "bs2 CONTROL: looked-up Bytes + CONSTANT start/len (no perform in the operands)"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def table (Map.insert Map.empty 1 (Bytes.of (list 10 20 30 40 50 60 70 80))))
                (handle St n
                  ((next (u) s (resume s (+ s 1))))
                  (match (Map.lookup table 1)
                    ((Some bs)
                      (+ (St.next)
                        (match (Bytes.slice bs 1 2)
                          ((Some sl) (* 10 (Bytes.len sl)))
                          ((None _u) -100))))
                    ((None _u) -200)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 21 Int64)))
