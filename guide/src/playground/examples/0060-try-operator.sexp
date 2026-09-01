(example
  (id "try-operator")
  (name "Fallible unwrap with try")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (safe-add a b) (let ((s (try (Int64.checked-add a b)))) (Some s)))

  (def (main) (safe-add 40 2))

  (export main)))
  (expected (: (Some 42) (Option Int64))))
