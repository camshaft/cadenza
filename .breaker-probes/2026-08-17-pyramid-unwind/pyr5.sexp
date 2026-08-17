(case "pyr5 a MATCH SCRUTINIZING THE RESUME CALL — each arm matches the resumed rest-of-body value mod three across three literal-guard arms adding a repdigit toll plus its own dispatch state, the inner frame's toll decides which arm the outer frame lands in, and the seed moves the inner frame across the arm boundary so the runs exit through different toll pairs"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (match (% (resume s (+ s 3)) 3)
                    (0 (+ 111 s))
                    (1 (+ 222 s))
                    (_ (+ 333 s)))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 223 Int64))
  (call   main (: 0 Int64)) (output (: 111 Int64)))
