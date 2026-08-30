(example
  (id "stack-via-prepend")
  (name "Stack (push via prepend)")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (push stack x) ((. List prepend) stack x))

  (def (main) (push (push (push (: #list() (List Int64)) 10) 20) 30))

  (export main)))
  (expected (: #list(30 20 10) (List Int64))))
