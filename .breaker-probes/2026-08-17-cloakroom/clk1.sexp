(case "clk1 a CLOAKROOM of numbered pegs — checking a coat takes the next ticket (answering ticket and rack size), a claim with a LIVE ticket returns the coat and clears the peg, a lost ticket charges a fee with the rack untouched, the read packs next-ticket rack and fees, and the seed pre-checks one coat so every ticket number and rack count shifts by one between the runs"
  (input  (do
            (effect C
              (op check (-> Int64 Int64))
              (op claim (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle C (if (> (% n 3) 0)
                            (tuple (Map.insert (: (Map.empty) (Map Int64 Int64)) 0 7) (: 1 Int64) (: 0 Int64))
                            (tuple (: (Map.empty) (Map Int64 Int64)) (: 0 Int64) (: 0 Int64)))
                ((check (item) st
                  (match st
                    ((tuple m nxt fees)
                      (resume (+ (* nxt 10) (Map.len (Map.insert m nxt item)))
                              (tuple (Map.insert m nxt item) (+ nxt 1) fees)))))
                 (claim (t) st
                  (match st
                    ((tuple m nxt fees)
                      (match (Map.lookup m t)
                        ((Some item) (resume (+ (* item 10) 1) (tuple (Map.remove m t) nxt fees)))
                        ((None) (resume (+ (: 900 Int64) (+ fees 1)) (tuple m nxt (+ fees 1))))))))
                 (read () st
                  (match st
                    ((tuple m nxt fees)
                      (resume (+ (* nxt 100) (+ (* (Map.len m) 10) fees)) st)))))
                (let ((a (C.check (: 5 Int64))))
                  (let ((b (C.claim (: 0 Int64))))
                    (let ((c (C.claim (: 2 Int64))))
                      (let ((d (C.check (: 9 Int64))))
                        (let ((f (C.read)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 12071901022321 Int64))
  (call   main (: 0 Int64)) (output (: 1051901011211 Int64)))
