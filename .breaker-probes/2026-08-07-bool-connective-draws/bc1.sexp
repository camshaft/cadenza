(case "bc1 short-circuit AND over two draws — the false-first row skips the second draw, observed by the branch draw"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (if (and (> (St.next) 2) (> (St.next) 4))
                    (St.next)
                    (- 0 (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7 Int64))
  (call   main (: 3 Int64)) (output (: -5 Int64))
  (call   main (: 0 Int64)) (output (: -1 Int64)))
