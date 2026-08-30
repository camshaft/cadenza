(example
  (id "effects-handlers-stateful")
  (name "Effects & handlers (stateful)")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (effect Tick (op tick (-> Int64 Int64)))

  (def
    (main)
    (handle Tick 0 ((tick (n) s (resume (+ s n) (+ s n)))) (+ ((. Tick tick) 10) ((. Tick tick) 5))))

  (export main)))
  (expected 25))
