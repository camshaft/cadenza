(case "mx4 a FOUR-param op mixing two Int64s, a NARROW UInt8, and a Bool — the narrow slot rides between wide ones and the flag doubles the first argument"
  (input  (do
            (effect E (op mix (-> Int64 UInt8 Bool Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((mix (a b flag d) s
                  (resume (+ (if flag (* a 2) a) (+ (Int64.of b) (+ (* d 100) s)))
                          (+ s 1))))
                (let ((x (E.mix 5 (UInt8.wrap 200) (= (% n 2) 1) 3)))
                  (let ((y (E.mix 5 (UInt8.wrap 200) (= (% n 2) 0) 3)))
                    (+ (* 10000 x) y)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 5130509 Int64))
  (call   main (: 4 Int64)) (output (: 5090515 Int64)))
