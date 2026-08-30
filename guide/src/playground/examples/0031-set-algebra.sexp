(example
  (id "set-algebra")
  (name "Set algebra (symmetric difference)")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def (sym-diff a b) ((. Set union) ((. Set difference) a b) ((. Set difference) b a)))

  (def
    (main)
    (let
      ((a ((. Set of) #list(1 2 3 4))) (b ((. Set of) #list(3 4 5 6))))
      ((. Set to-list) (sym-diff a b))))

  (export main)))
  (expected (: #list(1 2 5 6) (List Int64))))
