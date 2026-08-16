(case "bmv1 BOYER-MOORE majority vote — the (leader,votes) state deposes on exhausted votes, the challenger seed flips the winner"
  (input  (do
            (effect S (op vote (-> Int64 Int64)) (op lead (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple 0 0)
                ((vote (c) st
                  (match st
                    ((tuple leader votes)
                      (if (= c leader)
                          (resume votes (tuple leader (+ votes 1)))
                          (if (< votes 1)
                              (resume 0 (tuple c 1))
                              (resume votes (tuple leader (- votes 1))))))))
                 (lead () st (match st ((tuple l _v) (resume l st)))))
                (let ((_a (S.vote 7)))
                  (let ((_b (S.vote 7)))
                    (let ((_c (S.vote n)))
                      (let ((_d (S.vote n)))
                        (let ((_e (S.vote n)))
                          (S.lead))))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 7 Int64))
  (call   main (: 9 Int64)) (output (: 9 Int64)))
