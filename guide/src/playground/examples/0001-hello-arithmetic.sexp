(example
  (id "hello-arithmetic")
  (name "Hello, arithmetic")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (main) (+ 2 3))

  (export main)))
  (expected 5))
