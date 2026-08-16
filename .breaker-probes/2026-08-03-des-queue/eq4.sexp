(case "eq4 fork test"
  (input  (do
            (type Q (QNil) (QCons Int64 Int64 Q))
            (def (q-insert (: q Q) (: t Int64) (: lbl Int64))
              (match q
                ((QNil) (QCons t lbl (QNil)))
                ((QCons qt qlbl rest)
                  (if (< t qt)
                      (QCons t lbl q)
                      (QCons qt qlbl (q-insert rest t lbl))))))
            (def (main (: k Int64))
              (let ((base (q-insert (q-insert (QNil) 2 k) 4 2)))
                (let ((withA (q-insert base 3 7))
                      (withB (q-insert base 1 8)))
                  (match withA
                    ((QCons _t1 l1 (QCons _t2 l2 _r))
                      (match withB
                        ((QCons _t3 l3 _r2)
                          (+ l1 (+ (* 10 l2) (* 100 l3))))
                        (_ -1)))
                    (_ -2)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 871 Int64)))
