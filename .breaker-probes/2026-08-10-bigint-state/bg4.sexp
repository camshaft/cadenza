(case "bg4 BIGINT comparison in the arm routes tri-band verdicts — doubling walks past both thresholds, one row compares a genuine multi-limb value"
  (input  (do
            (effect E (op judge (-> Int64)))
            (def (main (: n Int64))
              (handle E (* (BigInt.of n) (BigInt.of 1000000000000000000))
                ((judge () s
                  (resume (if (> s (BigInt.of 5000000000000000000)) 2
                              (if (< s (- (BigInt.of 0) (BigInt.of 5000000000000000000))) 0 1))
                          (* s (BigInt.of 2)))))
                (+ (* 100 (E.judge)) (+ (* 10 (E.judge)) (E.judge)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 112 Int64))
  (call   main (: -2 Int64)) (output (: 110 Int64))
  (call   main (: 4 Int64)) (output (: 122 Int64)))
