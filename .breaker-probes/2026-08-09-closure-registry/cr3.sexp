(case "cr3 control: NON-recursive closure-composition (nested let, no fold)"
  (input  (do
            (def (main (: k Int64))
              (let ((f (fn ((: x Int64)) (+ x 1))))
                (let ((g (fn ((: x Int64)) (* (f x) 2))))
                  (let ((h (fn ((: x Int64)) (- (g x) 3))))
                    (h k)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9 Int64)))
