(case "ax2 scalar control for ax1"
  (input  (do
            (effect Bail (op stop (-> Int64 Int64)))
            (effect Log (op note (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Log 0
                ((note (v) s (resume s (+ s 1))))
                (+ (* 100 (Log.note n))
                   (+ (* 10 (handle Bail 0
                              ((stop (v) s (* v 2)))
                              (+ 999 (+ (Log.note 7) (Bail.stop 3)))))
                      (Log.note 8)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 62 Int64)))
