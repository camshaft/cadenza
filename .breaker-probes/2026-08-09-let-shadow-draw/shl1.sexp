(case "shl1 an inner let SHADOWS a draw binder — the shadow scales it locally, the original stays visible after the shadow closes"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (+ (let ((d (* d 100))) d)
                     (+ d (* 10 (- (E.probe) n)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 313 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64))
  (call   main (: -2 Int64)) (output (: -192 Int64)))
