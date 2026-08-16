(case "rk2 records reached VIA merge and VIA without dedupe with the direct build as ONE Set element"
  (input  (do
            (def (main (: n Int64))
              (do
                (def via-merge (Record.merge (record (a n)) (record (b 2))))
                (def via-without (Record.without (record (a n) (b 2) (c 9)) (c)))
                (Set.len (Set.of (list via-merge via-without (record (a n) (b 2)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
