(case "gl2 the let-lifted pure-guard equivalent of the gp2 cascade — both guard draws pre-bound, all three arms reachable"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (let ((k (St.next)))
                  (let ((g1 (St.next)))
                    (let ((g2 (St.next)))
                      (match k
                        ((guard _a (> g1 50)) 111)
                        ((guard _b (> g2 10)) (+ 200 (St.next)))
                        (_o (- 0 (St.next)))))))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 248 Int64))
  (call   main (: 30 Int64)) (output (: 111 Int64))
  (call   main (: 1 Int64)) (output (: -8 Int64)))
