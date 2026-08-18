(case "pyr9 a BARE-BINDER MATCH on the resume result — the single match arm binds whatever the resumed rest-of-body returned with no literal alternatives at all and answers it tripled-plus-state, the minimal match-scrutinee-binder shape, tripling per frame so the unwind pairing is pinned by magnitude"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (match (resume s (+ s 3))
                    (r (+ (* r 3) s)))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 382 Int64))
  (call   main (: 0 Int64)) (output (: 279 Int64)))
