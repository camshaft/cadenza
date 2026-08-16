(case "sf2 both same-effect handlers STATEFUL — the outer's advance survives the inner's forward"
  (input  (do
            (effect Ctr (op bump (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Ctr 100
                ((bump (v) t (resume (+ v t) (+ t 1))))
                (+ (handle Ctr 0
                     ((bump (v) s (resume (Ctr.bump (* v 10)) (+ s 1))))
                     (+ (Ctr.bump n) (Ctr.bump 1)))
                   (Ctr.bump 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 365 Int64)))
