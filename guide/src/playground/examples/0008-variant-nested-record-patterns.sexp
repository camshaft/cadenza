(example
  (id "variant-nested-record-patterns")
  (name "Variant-nested record patterns (match through a variant)")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def
    (dist opt)
    (match opt ((Some #record((= x px) (= y py))) (+ (* px px) (* py py))) ((None) 0)))

  (def (main) (dist (Some #record((= x 3) (= y 4)))))

  (export main)))
  (expected 25))
