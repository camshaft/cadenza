(case "ha1 a FORWARDING interposer: middle handler re-performs the outer op unchanged (pass-through adapter)"
  (input  (do
            (effect Base (op ask (-> Unit Int64)))
            (effect Wrap (op ask (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Base n
                ((ask (u) s (resume s (+ s 1))))
                (handle Wrap 0
                  ((ask (u) t (resume (Base.ask) t)))
                  (+ (* 10 (Wrap.ask)) (Wrap.ask)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
