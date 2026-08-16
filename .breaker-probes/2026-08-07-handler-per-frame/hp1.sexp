(case "hp1 a RECURSIVE fn installs a fresh same-effect handler per frame — the base case draws from the deepest, each frame from its own"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (level (: d Int64))
              (if (<= d 0)
                  (St.next)
                  (handle St (* d 100)
                    ((next () s (resume s (+ s 1))))
                    (+ (St.next) (level (- d 1))))))
            (def (main (: n Int64))
              (handle St 7
                ((next () s (resume s (+ s 1))))
                (level n)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 401 Int64))
  (call   main (: 0 Int64)) (output (: 7 Int64))
  (call   main (: 3 Int64)) (output (: 701 Int64)))
