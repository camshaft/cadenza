(case "hp3 the recursive call sits in the SEED — each frame's handler is seeded by the whole subtree below it"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (level (: d Int64))
              (if (<= d 0)
                  (St.next)
                  (handle St (level (- d 1))
                    ((next () s (resume s (+ s 1))))
                    (+ (St.next) (St.next)))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (level 2)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 23 Int64))
  (call   main (: 3 Int64)) (output (: 15 Int64)))
