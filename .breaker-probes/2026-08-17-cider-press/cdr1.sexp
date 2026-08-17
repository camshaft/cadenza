(case "cdr1 a CIDER press splitting juice from pomace — pressing yields two-thirds of the hopper as juice (integer) with the REMAINDER going to pomace and the hopper emptying (a dry press answers nine-hundred with the pomace's low digit touching nothing), loading adds apples, the read packs juice hopper and pomace, and the seed's first pressing yields four-to-one on the full hopper against one-to-one on the meagre one with the split ratios diverging every round"
  (input  (do
            (effect C
              (op load (-> Int64 Int64))
              (op press (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle C (tuple (+ (: 2 Int64) (* (% n 3) 5)) (: 0 Int64) (: 0 Int64))
                ((load (k) st
                  (match st
                    ((tuple hop juice pom)
                      (resume (+ (* (+ hop k) 10) (% k 10)) (tuple (+ hop k) juice pom)))))
                 (press () st
                  (match st
                    ((tuple hop juice pom)
                      (if (= (/ (* hop 2) 3) 0)
                          (resume (+ (: 900 Int64) (% pom 10)) st)
                          (resume (+ (* (/ (* hop 2) 3) 10)
                                     (% (+ pom (- hop (/ (* hop 2) 3))) 10))
                                  (tuple (: 0 Int64)
                                         (+ juice (/ (* hop 2) 3))
                                         (+ pom (- hop (/ (* hop 2) 3)))))))))
                 (read () st
                  (match st
                    ((tuple hop juice pom)
                      (resume (+ (* juice 100) (+ (* hop 10) pom)) st)))))
                (let ((a (C.press)))
                  (let ((b (C.load (: 4 Int64))))
                    (let ((c (C.press)))
                      (let ((d (C.press)))
                        (let ((f (C.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 430440259050605 Int64))
  (call   main (: 0 Int64)) (output (: 110440239030303 Int64)))
