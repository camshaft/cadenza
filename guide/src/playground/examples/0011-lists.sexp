(example
  (id "lists")
  (name "Lists")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (main) ((. List concat) #list(1 2) #list(3 4 5)))

  (export main)))
  (expected (: #list(1 2 3 4 5) (List Int64))))
