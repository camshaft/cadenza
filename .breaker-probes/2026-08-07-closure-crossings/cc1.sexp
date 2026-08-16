(case "cc1 a closure over the fn PARAM built before the handle, applied twice inside with draws — capture stable, draws advance"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (let ((f (fn ((: x Int64)) (* x n))))
                (handle St 3
                  ((next () s (resume s (+ s 2))))
                  (+ (f (St.next)) (f (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 40 Int64))
  (call   main (: 2 Int64)) (output (: 16 Int64)))
