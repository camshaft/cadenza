(case "dd1d a bare DISCARDED draw before a let-bound draw — the discard advances the state the binder reads"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (do
                  (St.next)
                  (let ((a (St.next)))
                    (+ (* 100 a) (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1020 Int64))
  (call   main (: 1 Int64)) (output (: 204 Int64)))
