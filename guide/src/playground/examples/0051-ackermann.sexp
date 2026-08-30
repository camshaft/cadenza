(example
  (id "ackermann")
  (name "Ackermann function")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def (ack m n) (if (= m 0) (+ n 1) (if (= n 0) (ack (- m 1) 1) (ack (- m 1) (ack m (- n 1))))))

  (def (main) (ack 3 3))

  (export main)))
  (expected 61))
