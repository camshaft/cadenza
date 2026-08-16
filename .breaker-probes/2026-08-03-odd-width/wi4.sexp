(case "wi4 odd-width values as CHAMP Set keys dedupe at their own width"
  (input  (do
            (def (main (: k Int64))
              (Set.len (Set.of (list ((. (UInt 4) wrap) k)
                                     ((. (UInt 4) wrap) (+ k 16))
                                     ((. (UInt 4) wrap) (+ k 1))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 2 Int64)))
