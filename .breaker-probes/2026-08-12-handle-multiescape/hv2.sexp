(case "hv2 control: ONE closure capturing performs escapes the handle"
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (do
                (def f
                  (handle Ctr k ((tick (u) s (resume s (+ s 1))))
                    (let ((a (Ctr.tick)))
                      (fn ((: x Int64)) (+ x a)))))
                (f 1)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))
