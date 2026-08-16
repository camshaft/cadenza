(case "ed3 a RECURSIVE def that handles per frame, called from a handled main — the base case draws from the deepest frame's handler"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (tower (: d Int64))
              (if (<= d 0)
                  (St.next)
                  (handle St (* d 1000)
                    ((next () s (resume s (+ s 1))))
                    (+ (St.next) (tower (- d 1))))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 3))))
                (+ (St.next) (tower 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4006 Int64))
  (call   main (: 0 Int64)) (output (: 4001 Int64)))
