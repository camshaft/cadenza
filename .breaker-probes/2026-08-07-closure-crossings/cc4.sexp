(case "cc4 TWO closures over SEQUENTIAL draws inside one region — each captures its own read, applied after both bind"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((a (St.next)))
                  (let ((f (fn ((: x Int64)) (+ x a))))
                    (let ((b (St.next)))
                      (let ((g (fn ((: x Int64)) (* x b))))
                        (+ (f 100) (g 10))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 165 Int64))
  (call   main (: 0 Int64)) (output (: 110 Int64)))
