(case "bc2 monotonicity of THREE draws under a parity-dependent step — the and of two comparisons decides, the final state rides along"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (if (= (% s 2) 0) (+ s 2) (- s 3))))
                 (probe () s (resume s s)))
                (let ((a (E.next)))
                  (let ((b (E.next)))
                    (let ((c (E.next)))
                      (+ (if (and (< a b) (< b c)) 1000 2000) (E.probe)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1006 Int64))
  (call   main (: 1 Int64)) (output (: 2002 Int64))
  (call   main (: 4 Int64)) (output (: 1010 Int64))
  (call   main (: -5 Int64)) (output (: 1996 Int64)))
