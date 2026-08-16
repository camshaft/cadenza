(case "rp1 a record pattern destructures a row-op DERIVED record (with-chain result)"
  (input  (do
            (def (main (: n Int64))
              (do
                (def base (record (a n) (b 2) (c 3)))
                (def derived (Record.with (Record.with base #"a" 10) #"c" 30))
                (match derived
                  ((record (a x) (c z)) (+ (* 100 x) z)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1030 Int64)))
