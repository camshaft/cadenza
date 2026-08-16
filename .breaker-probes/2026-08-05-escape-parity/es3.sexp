(case "es3 the closure's effect is discharged INSIDE the closure body — no escape, must run"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (host (ask)
                ((fn (x) (handle Ctr 7 ((tick (u) s (resume s s))) (+ x (Ctr.tick)))) k)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 10 Int64)))
