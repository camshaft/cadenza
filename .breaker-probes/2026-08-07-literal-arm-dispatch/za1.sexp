(case "za1 literal-arm dispatch on a draw — the MATCHED arm performs again, both calls exercised"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((k (St.next)))
                  (match k
                    (5 (+ 100 (St.next)))
                    (6 200)
                    (_o 300)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106 Int64))
  (call   main (: 6 Int64)) (output (: 200 Int64))
  (call   main (: 9 Int64)) (output (: 300 Int64)))
