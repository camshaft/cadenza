(case "neu1 KAHAN COMPENSATED SUMMATION as a handler state — the (sum,comp) pair recovers a small addend that naive summation absorbs at the 2^53 boundary, the naive control confirms the absorption"
  (input  (do
            (effect S (op feed (-> Float64 Float64)))
            (def (main (: n Int64))
              (handle S (tuple 0.0 0.0)
                ((feed (v) st
                  (match st
                    ((tuple sm comp)
                      (let ((y (- v comp)))
                        (let ((t (+ sm y)))
                          (resume t (tuple t (- (- t sm) y)))))))))
                (let ((big 9007199254740992.0))
                  (let ((_a (S.feed big)))
                    (let ((_b (S.feed (Float64.of-int n))))
                      (let ((k (S.feed (- 0.0 big))))
                        (let ((naive (- (+ big (Float64.of-int n)) big)))
                          (+ (* 10.0 k) naive))))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 10.0 Float64))
  (call   main (: 5 Int64)) (output (: 54.0 Float64)))
