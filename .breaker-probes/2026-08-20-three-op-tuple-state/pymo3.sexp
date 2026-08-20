(case "pymo3 probe: a THREE-OP effect over a TUPLE state where each op touches a different field — adda answers a and threads (a+1, b), addb answers b and threads (a, b+10), rd answers a*100+b and threads unchanged; the body interleaves adda/addb/adda/addb/rd so three distinct ops read and independently advance the two fields of one shared tuple state through the tail-resumptive fold"
  (input (do
  (effect E (op adda (-> Int64)) (op addb (-> Int64)) (op rd (-> Int64)))
  (def (main (: n Int64))
    (handle E (tuple (% n 3) (: 0 Int64))
      ((adda () s (match s ((tuple a b) (resume a (tuple (+ a 1) b)))))
       (addb () s (match s ((tuple a b) (resume b (tuple a (+ b 10))))))
       (rd () s (match s ((tuple a b) (resume (+ (* a 100) b) s)))))
      (+ (* 10000 (E.adda)) (+ (* 1000 (E.addb)) (+ (* 100 (E.adda)) (+ (* 10 (E.addb)) (E.rd)))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 10620 Int64))
  (call   main (: 0 Int64)) (output (: 420 Int64)))
