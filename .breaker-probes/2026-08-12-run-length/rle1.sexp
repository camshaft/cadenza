(case "rle1 RUN-LENGTH tracking — the 3-tuple state carries (last,run,best); the n=5 seed extends one run to five, n=7 breaks it at two"
  (input  (do
            (effect S (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple 0 0 0)
                ((feed (v) st
                  (match st
                    ((tuple last run best)
                      (let ((nrun (if (= v last) (+ run 1) 1)))
                        (resume nrun (tuple v nrun (if (> nrun best) nrun best))))))))
                (let ((_a (S.feed 5)))
                  (let ((_b (S.feed 5)))
                    (let ((_c (S.feed n)))
                      (let ((_d (S.feed n)))
                        (S.feed n)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64))
  (call   main (: 7 Int64)) (output (: 3 Int64)))
