(case "pp3 pipe stages are evaluated left-to-right with effects (each stage's perform in order)"
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (stage1 (: v Int64)) (+ v (* 100 (Ctr.tick))))
            (def (stage2 (: v Int64)) (+ v (* 10000 (Ctr.tick))))
            (def (main (: k Int64))
              (handle Ctr 1 ((tick (u) s (resume s (+ s 1))))
                (|> (|> k stage1) stage2)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 20105 Int64)))
