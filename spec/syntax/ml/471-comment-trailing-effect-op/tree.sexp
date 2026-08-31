(do
  (effect
    E
    (comment-after "note on get" (op get (-> Int64 Int64)))
    (comment-after "note on put" (op put (-> Int64 Unit))))

  (def (f) 1))
