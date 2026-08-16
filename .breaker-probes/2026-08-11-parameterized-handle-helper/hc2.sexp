(case "hc2 a def-wrapped handle with a PARAMETER seed called twice — each region's recursive draws start from its own seed"
  (input  (do
            (effect A (op get (-> Int64)))
            (def (wk (: k Int64))
              (if (< k 1) 0 (let ((d (A.get))) (+ d (wk (- k 1))))))
            (def (region (: seed Int64) (: n Int64))
              (handle A seed
                ((get () s (resume s (+ s 1))))
                (wk n)))
            (def (main (: n Int64))
              (+ (region 5 n) (* 1000 (region 70 n))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 213018 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
