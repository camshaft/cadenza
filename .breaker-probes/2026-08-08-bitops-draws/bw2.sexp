(case "bw2 an XOR accumulator folds four draws through a recursion — bit-mixing order-sensitive under the stride"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (mix (: k Int64) (: acc Int64))
              (if (<= k 0) acc (mix (- k 1) (^ acc (E.next)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 3))))
                (+ (* 10 (mix 4 0)) 12)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 92 Int64))
  (call   main (: 0 Int64)) (output (: 132 Int64))
  (call   main (: 9 Int64)) (output (: 252 Int64)))
