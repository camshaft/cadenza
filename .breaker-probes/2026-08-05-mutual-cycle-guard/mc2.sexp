(case "mc2 mutual recursion where each fn's perform feeds the OTHER's continuation (deeper cycle shape)"
  (input  (do
            (effect St (op ping (-> Int64 Int64)))
            (def (even-w (: n Int64) (: acc Int64))
              (if (= n 0) acc (odd-w (- n 1) (+ acc (St.ping n)))))
            (def (odd-w (: n Int64) (: acc Int64))
              (if (= n 0) acc (even-w (- n 1) (+ acc (* 100 (St.ping n))))))
            (def (main (: k Int64))
              (handle St 0
                ((ping (v) s (resume v (+ s 1))))
                (even-w k 0)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 406 Int64)))
