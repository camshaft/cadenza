(case "ev3 eval result feeds a CHAMP key and the quote value itself is a Map key"
  (input  (do
            (def (main (: k Int64))
              (+ (* 10 (match (Map.lookup (Map.insert Map.empty (eval (quote (+ 5 5))) 42) 10)
                         ((Some v) v) ((None _u) -1)))
                 (match (Map.lookup (Map.insert Map.empty (quote (+ 1 k)) 9) (quote (+ 1 k)))
                   ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 429 Int64)))
