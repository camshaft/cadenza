(example
  (id "a-type-error")
  (name "A type error (see the squiggle)")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (main) (+ 1 true))

  (export main)))
  (expect-error "true"))
