(example
  (id "higher-order-apply-n")
  (name "Higher-order functions (apply n times)")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (apply-n-times f n x) (if (= n 0) x (apply-n-times f (- n 1) (f x))))

  (def (adder k) (fn (x) (+ x k)))

  (def (main) #tuple((apply-n-times (adder 3) 4 10) (apply-n-times (fn (x) (* x 2)) 5 1)))

  (export main)))
  (expected (: #tuple(22 32) (Tuple Int64 Int64))))
