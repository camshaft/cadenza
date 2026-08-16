(case "be2 a perform result LET-bound then fed to a bin segment folds"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((v (UInt8.wrap (St.next))))
                  (match (Bytes.at (bin (u8 v)) 0)
                    ((Some b) (Int64.of b))
                    ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
