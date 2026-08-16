(case "ng2 Int64 MAX threads the handler state intact (representation at the boundary)"
  (input  (do
            (effect St (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 9223372036854775807
                ((peek (u) s (resume s (- s 1))))
                (if (= (St.peek) 9223372036854775807) (if (= (St.peek) 9223372036854775806) 1 2) 3)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
