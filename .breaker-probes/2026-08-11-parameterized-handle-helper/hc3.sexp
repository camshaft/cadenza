(case "hc3 CHAINED parameterized regions — the first region's drained result seeds the second, same def both times"
  (input  (do
            (effect A (op get (-> Int64)))
            (def (wk (: k Int64))
              (if (< k 1) 0 (let ((d (A.get))) (+ d (wk (- k 1))))))
            (def (region (: seed Int64) (: n Int64))
              (handle A seed
                ((get () s (resume s (+ s 1))))
                (wk n)))
            (def (main (: n Int64))
              (region (region 2 n) n))
            (export main)))
  (call   main (: 3 Int64)) (output (: 30 Int64))
  (call   main (: 1 Int64)) (output (: 2 Int64)))
