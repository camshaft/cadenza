(example
  (id "pattern-matching")
  (name "Pattern matching")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (main) (match 2 (1 10) (2 20) (_ 0)))

  (export main)))
  (expected 20))
