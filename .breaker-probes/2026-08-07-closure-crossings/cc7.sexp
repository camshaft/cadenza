(case "cc7 a draw-capturing closure threaded through a RECURSIVE helper — applied per frame, the leaf applies it to a fresh draw"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (walk (: d Int64) (: f (-> Int64 Int64)))
              (if (<= d 0)
                  (f (St.next))
                  (+ (f d) (walk (- d 1) f))))
            (def (main (: n Int64))
              (handle St 100
                ((next () s (resume s (+ s 1))))
                (let ((k (St.next)))
                  (walk n (fn ((: x Int64)) (* x k))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 10700 Int64))
  (call   main (: 0 Int64)) (output (: 10100 Int64)))
