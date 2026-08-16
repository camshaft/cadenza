(case "sl1 a DEPTH-3 same-effect shadow ladder — each shadow's seed draws from the ENCLOSING handler, strides 1 2 3 stay separate"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (handle St (* (St.get) 10)
                  ((get () t (resume t (+ t 2))))
                  (handle St (+ (St.get) 5)
                    ((get () u (resume u (+ u 3))))
                    (+ (St.get) (* 100 (St.get)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3835 Int64))
  (call   main (: 0 Int64)) (output (: 805 Int64)))
