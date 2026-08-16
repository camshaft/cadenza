(case "bwa1 THREE parallel bit-accumulators in one state — running AND, OR, and XOR folds over the drawn payloads, read as a sum"
  (input  (do
            (effect S (op mix (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple 0 0 0)
                ((mix (v) st
                  (match st
                    ((tuple ao oo xo)
                      (resume (+ ao (+ oo xo))
                              (tuple (& (if (= ao 0) v ao) v) (| oo v) (^ xo v)))))))
                (let ((_a (S.mix 12)))
                  (let ((_b (S.mix 10)))
                    (let ((_c (S.mix n)))
                      (S.mix 0))))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 14 Int64))
  (call   main (: 15 Int64)) (output (: 32 Int64)))
