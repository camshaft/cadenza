(example
  (id "nested-record-patterns")
  (name "Nested record patterns (destructure in place)")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def
    (weigh pair)
    (match pair (#tuple(#record((= x px) (= y py)) w) (* w (+ (* px px) (* py py))))))

  (def (main) (weigh #tuple(#record((= x 3) (= y 4)) 2)))

  (export main)))
  (expected 50))
