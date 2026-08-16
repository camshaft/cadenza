(case "hp2 per-frame handlers with a POST-ORDER draw — each frame draws its own state AFTER the recursive call returns"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (level (: d Int64))
              (if (<= d 0)
                  (St.next)
                  (handle St (* d 100)
                    ((next () s (resume s (+ s 1))))
                    (+ (level (- d 1)) (* 1000 (St.next))))))
            (def (main (: n Int64))
              (handle St 7
                ((next () s (resume s (+ s 1))))
                (level n)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 301100 Int64))
  (call   main (: 1 Int64)) (output (: 101100 Int64)))
