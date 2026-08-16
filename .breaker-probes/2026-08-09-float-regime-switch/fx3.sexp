(case "fx3 a FLOAT slot with a regime-switch beside an INT counter in one tuple state — mixed-width slots thread independently"
  (input  (do
            (effect E (op draw (-> Float64)) (op count (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple (+ 1.0 (Float64.of-int n)) 0)
                ((draw () st (match st
                               ((tuple s c)
                                (resume s (tuple (if (< s 10.0) (* s 2.0) (* s 0.5)) (+ c 1))))))
                 (count () st (match st ((tuple s c) (resume c st)))))
                (+ (E.draw) (+ (E.draw) (+ (E.draw) (Float64.of-int (E.count)))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 24.0 Float64))
  (call   main (: 0 Int64)) (output (: 10.0 Float64))
  (call   main (: 5 Int64)) (output (: 27.0 Float64)))
