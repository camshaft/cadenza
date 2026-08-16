(case "fl1 halve-then-double round trips through TWO dispatches — exact binary floats compare equal after the arm trip"
  (input  (do
            (effect E (op halve (-> Float64 Float64)) (op dbl (-> Float64 Float64)) (op count (-> Float64)))
            (def (main (: u Float64))
              (handle E 0.0
                ((halve (x) s (resume (* x 0.5) (+ s 1.0)))
                 (dbl (x) s (resume (* x 2.0) (+ s 1.0)))
                 (count () s (resume s s)))
                (+ (if (= (E.dbl (E.halve 3.0)) 3.0) 100.0 900.0)
                   (+ (if (= (E.dbl (E.halve 0.75)) 0.75) 10.0 90.0)
                      (E.count)))))
            (export main)))
  (call   main (: 0.0 Float64)) (output (: 114.0 Float64)))
