(case "spn1 a SPINNING WHEEL twisting yarn — drafting four or more twist converts HALF to yarn keeping the remainder on the bobbin (the div/mod pair over one field with the quotient in the answer and the remainder threading), a thin draft TANGLES (counted, the twist lost to zero), treadling adds twist, the read packs yarn twist and tangles, and the seed's starting twist drafts clean-treadle-clean-tangle on one wheel against tangle-treadle-clean-tangle on the other"
  (input  (do
            (effect S
              (op treadle (-> Int64 Int64))
              (op draft (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple (+ (: 2 Int64) (* (% n 3) 3)) (: 0 Int64) (: 0 Int64))
                ((treadle (t) st
                  (match st
                    ((tuple tw yarn tg)
                      (resume (+ (* (+ tw t) 10) (% t 10)) (tuple (+ tw t) yarn tg)))))
                 (draft () st
                  (match st
                    ((tuple tw yarn tg)
                      (if (>= tw 4)
                          (resume (+ (* (/ tw 2) 10) (% tw 2))
                                  (tuple (% tw 2) (+ yarn (/ tw 2)) tg))
                          (resume (+ (: 900 Int64) (+ tg 1))
                                  (tuple (: 0 Int64) yarn (+ tg 1)))))))
                 (read () st
                  (match st
                    ((tuple tw yarn tg)
                      (resume (+ (* yarn 100) (+ (* tw 10) tg)) st)))))
                (let ((a (S.draft)))
                  (let ((b (S.treadle (: 5 Int64))))
                    (let ((c (S.draft)))
                      (let ((d (S.draft)))
                        (let ((f (S.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 210650309010501 Int64))
  (call   main (: 0 Int64)) (output (: 9010550219020202 Int64)))
