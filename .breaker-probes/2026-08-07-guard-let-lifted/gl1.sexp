(case "gl1 the let-lifted pure-guard equivalent of the gp1 dispatch shape — the pre-bound guard draw advances the state the arms read"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((k (St.next)))
                  (let ((g (St.next)))
                    (match k
                      ((guard _x (> g 6)) (+ 100 (St.next)))
                      (_o (* 10 (St.next))))))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 108 Int64))
  (call   main (: 2 Int64)) (output (: 40 Int64)))
