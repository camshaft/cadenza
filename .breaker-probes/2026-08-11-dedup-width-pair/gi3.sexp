(case "gi3 two recursive performers CONGRUENT except payload width — Int64 and Float64 accumulators must not merge across width"
  (input  (do
            (effect A (op get (-> Int64)))
            (def (walki (: acc Int64) (: k Int64))
              (if (< k 1) acc (walki (+ acc (A.get)) (- k 1))))
            (def (walkf (: acc Float64) (: k Int64))
              (if (< k 1) acc (walkf (+ acc (Float64.of-int (A.get))) (- k 1))))
            (def (main (: n Int64))
              (handle A 10
                ((get () s (resume s (+ s 1))))
                (+ (* 100 (walki 0 n))
                   (if (= (walkf 0.5 n) 39.5) 7 8))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3308 Int64))
  (call   main (: 0 Int64)) (output (: 8 Int64)))
