(case "hi2 the arm-installed handle's body draws from the arm's ENCLOSING frame — fresh inner frame and outer dispatch in one arm"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect M (op grab (-> Int64)))
            (effect J (op get (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle M 0
                  ((grab () m (resume (handle J 9
                                         ((get () t (resume t t)))
                                         (+ (J.get) (O.next)))
                                       m)))
                  (+ (* 10 (M.grab)) (O.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 146 Int64))
  (call   main (: 0 Int64)) (output (: 91 Int64))
  (call   main (: -3 Int64)) (output (: 58 Int64)))
