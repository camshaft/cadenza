(do
  (type
    T
    (comment-after "trailing on A" (A Int64))
    (comment-after "trailing on B" (comment "continuation of A" (B Int64))))

  (def (f) 1))
