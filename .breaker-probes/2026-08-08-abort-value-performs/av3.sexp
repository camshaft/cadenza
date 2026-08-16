(case "av3 in a THREE-stack the innermost arm aborts with a MIDDLE draw — the escaping value advances the middle thread, later draws see it"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect M (op step (-> Int64)))
            (effect Bail (op out (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle M 4
                  ((step () m (resume m (+ m 2))))
                  (+ (* 100 (handle Bail 0
                              ((out () t (M.step)))
                              (+ (Bail.out) 999)))
                     (+ (* 10 (M.step)) (O.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 465 Int64))
  (call   main (: 0 Int64)) (output (: 460 Int64))
  (call   main (: -7 Int64)) (output (: 453 Int64)))
