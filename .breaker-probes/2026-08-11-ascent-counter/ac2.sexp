(case "ac2 the ascent counter fed by a SECOND effect's squared draws through a recursive driver — the negative seed dips at the parabola turn"
  (input  (do
            (effect W (op feed (-> Int64 Int64)))
            (effect G (op next (-> Int64)))
            (def (drive (: k Int64) (: acc Int64))
              (if (< k 1) acc (drive (- k 1) (+ acc (W.feed (G.next))))))
            (def (main (: n Int64))
              (handle G n
                ((next () g (resume (* g g) (+ g 1))))
                (handle W (tuple 0 0)
                  ((feed (v) st
                    (match st
                      ((tuple prev hits)
                        (let ((nh (if (< prev v) (+ hits 1) hits)))
                          (resume nh (tuple v nh)))))))
                  (drive 4 0))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 10 Int64))
  (call   main (: -2 Int64)) (output (: 5 Int64)))
