(case "rk3 a record reached via a with-CHAIN (three generations) keys like the final direct build"
  (input  (do
            (def (main (: n Int64))
              (do
                (def g0 (record (a n) (b n) (c n)))
                (def g3 (Record.with (Record.with (Record.with g0 #"a" 1) #"b" 2) #"c" 3))
                (+ (* 10 (match (Map.lookup (Map.insert Map.empty (record (a 1) (b 2) (c 3)) 7) g3)
                           ((Some v) v) ((None _u) -1)))
                   (. g0 a))))
            (export main)))
  (call   main (: 9 Int64)) (output (: 79 Int64)))
