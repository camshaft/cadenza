(example
  (id "records")
  (name "Records")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (area r) (* (. r w) (. r h)))

  (def (main) (area #record((= w 4) (= h 5))))

  (export main)))
  (expected 20))
