(do
  (def (f x) (match x (comment-after "zero" (0 1)) (comment-after "other" (_ 2))))

  (def (g) 9))
