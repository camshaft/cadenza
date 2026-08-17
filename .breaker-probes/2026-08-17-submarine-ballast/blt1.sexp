(case "blt1 a SUBMARINE ballast trim — flooding adds ballast and dives at double rate CAPPED at twenty (a nine-tagged crush row), blowing vents the LESSER of the request and the ballast rising at double rate where hitting the surface BREACHES (counted, seven-hundred row), the read packs depth ballast and breaches, and the seed's standing ballast lets one boat ride the full drill submerged while the other's final blow breaches the surface"
  (input  (do
            (effect S
              (op flood (-> Int64 Int64))
              (op blow (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple (* (* (% n 3) 2) 2) (* (% n 3) 2) (: 0 Int64))
                ((flood (k) st
                  (match st
                    ((tuple depth ballast breaches)
                      (if (> (+ depth (* k 2)) 20)
                          (resume (+ (: 200 Int64) 9)
                                  (tuple (: 20 Int64) (+ ballast k) breaches))
                          (resume (+ (* (+ depth (* k 2)) 10) (% (+ ballast k) 10))
                                  (tuple (+ depth (* k 2)) (+ ballast k) breaches))))))
                 (blow (k) st
                  (match st
                    ((tuple depth ballast breaches)
                      (if (< k ballast)
                          (if (<= (- depth (* k 2)) 0)
                              (resume (+ (: 700 Int64) (+ breaches 1))
                                      (tuple (: 0 Int64) (- ballast k) (+ breaches 1)))
                              (resume (+ (* (- depth (* k 2)) 10) k)
                                      (tuple (- depth (* k 2)) (- ballast k) breaches)))
                          (if (<= (- depth (* ballast 2)) 0)
                              (resume (+ (: 700 Int64) (+ breaches 1))
                                      (tuple (: 0 Int64) (: 0 Int64) (+ breaches 1)))
                              (resume (+ (* (- depth (* ballast 2)) 10) ballast)
                                      (tuple (- depth (* ballast 2)) (: 0 Int64) breaches)))))))
                 (read () st
                  (match st
                    ((tuple depth ballast breaches)
                      (resume (+ (* depth 100) (+ (* ballast 10) breaches)) st)))))
                (let ((a (S.flood (: 4 Int64))))
                  (let ((b (S.blow (: 2 Int64))))
                    (let ((c (S.flood (: 6 Int64))))
                      (let ((d (S.blow (: 9 Int64))))
                        (let ((f (S.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1260822000290210 Int64))
  (call   main (: 0 Int64)) (output (: 840421687010001 Int64)))
