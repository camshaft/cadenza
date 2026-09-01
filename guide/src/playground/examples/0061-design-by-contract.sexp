(example
  (id "design-by-contract")
  (name "Design by contract")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (@ (requires (>= x 0)) (@ (ensures (>= ret 0)) (def (double (: x Int64)) (* x 2))))

  (def (main) (double 21))

  (export main)))
  (expected 42))
