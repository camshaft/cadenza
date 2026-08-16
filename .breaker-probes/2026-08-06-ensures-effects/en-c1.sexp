(do
  (@ (ensures (>= ret 0)) (def (dbl (: x Int64)) (+ x x)))
  (def (main (: n Int64)) (dbl n))
  (export main))
