(case "rk1 a record reached VIA Record.with keys a Map like the directly-built record"
  (input  (do
            (def (main (: n Int64))
              (do
                (def base (record (a n) (b 2)))
                (def derived (Record.with base #"a" 5))
                (+ (* 10 (match (Map.lookup (Map.insert Map.empty (record (a 5) (b 2)) 42) derived)
                           ((Some v) v) ((None _u) -1)))
                   (if (= derived (record (a 5) (b 2))) 1 0))))
            (export main)))
  (call   main (: 9 Int64)) (output (: 421 Int64)))
