(case "pyx2 a POKE OVERWRITES THE THREAD FROM ITS ARGUMENT — the ticks advance the state incrementally while a poke answers the OLD state and replaces the thread wholesale with its argument, the third dispatch reads the transplanted seven rather than any incremental descendant, and a poke that merges instead of replacing or answers the new value shifts separate digit ranges"
  (input  (do
            (effect E (op tick (-> Int64)) (op poke (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (resume (* s 10) (+ s 1)))
                 (poke (v) s (resume s v)))
                (+ (E.tick)
                   (+ (* 10 (E.poke 7))
                      (* 1000 (E.tick))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 70030 Int64))
  (call   main (: 0 Int64)) (output (: 70010 Int64)))
