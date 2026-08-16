(do
  (effect ask (op ask (-> Unit Int64)))
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (host (ask)
      (handle St (ask.ask)
        ((next (u) s (resume s (+ s (ask.ask)))))
        (+ (* 10 (St.next)) (St.next)))))
  (export main))
