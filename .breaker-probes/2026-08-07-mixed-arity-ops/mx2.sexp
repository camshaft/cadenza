(case "mx2 mixed-arg op with BOTH args draw-derived — the int arg is a draw, the string arg branches on a second draw"
  (input  (do
            (effect E (op pick (-> Int64)) (op mix (-> Int64 String Int64)))
            (def (main (: n Int64))
              (handle E n
                ((pick () s (resume s (+ s 2)))
                 (mix (k w) s (resume (+ (* k (String.byte-len w)) s) s)))
                (E.mix (E.pick) (if (> (E.pick) 6) "wide" "nn"))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 29 Int64))
  (call   main (: 1 Int64)) (output (: 7 Int64)))
