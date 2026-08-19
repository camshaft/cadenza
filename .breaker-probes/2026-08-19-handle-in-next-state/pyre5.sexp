(case "pyre5 a MATCH-over-handle in next-state — the even-state arm selects the closed pure handle (miscompiles pre-fix, correct 46200) while the odd-state arm selects a pure constant nine (folds correctly), completing the neighborhood sweep alongside if (pyre4) and let: the decline guard recurses the whole next-state subtree so all wrapper shapes reject rather than miscompile" (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (* s 10)
                   (match (% s 2)
                     (0 (handle E (: 40 Int64) ((tick () t (resume t (+ t 1)))) (+ (E.tick) 2)))
                     (_ (: 9 Int64))))
           (* 1000 s))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 10910 Int64))
  (call   main (: 0 Int64)) (output (: 46200 Int64)))
