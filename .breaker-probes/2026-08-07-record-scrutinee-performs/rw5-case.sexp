(case "rw5 let-bound record scrutinee control — bind first, then match"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((r (record (a (St.next)) (b (St.next)))))
                  (match r ((record (a x) (b y)) (+ (* 10 x) y))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
