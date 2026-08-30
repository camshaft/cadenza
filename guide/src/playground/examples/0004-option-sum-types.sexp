(example
  (id "option-sum-types")
  (name "Option & sum types")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (type Opt (Some Int64) (None unit))

  (def (main) (match (Some 7) ((Some x) x) ((None _) 0)))

  (export main)))
  (expected 7))
