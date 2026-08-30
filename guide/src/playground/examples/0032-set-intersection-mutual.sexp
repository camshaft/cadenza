(example
  (id "set-intersection-mutual")
  (name "Mutual friends (set intersection)")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def (mutual a b) ((. Set to-list) ((. Set intersection) ((. Set of) a) ((. Set of) b))))

  (def (main) (mutual #list(1 2 3 4 5) #list(3 4 5 6 7)))

  (export main)))
  (expected (: #list(3 4 5) (List Int64))))
