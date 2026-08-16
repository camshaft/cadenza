(case "bc2 short-circuit OR over two draws — the true-first row skips the second draw"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (if (or (> (St.next) 4) (> (St.next) 1))
                    (St.next)
                    (- 0 (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64))
  (call   main (: 2 Int64)) (output (: 4 Int64))
  (call   main (: 0 Int64)) (output (: -2 Int64)))
