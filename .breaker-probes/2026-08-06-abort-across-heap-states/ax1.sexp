(case "ax1 an inner abort discards frames whose HEAP states were mid-mutation — the outer heap state reads intact"
  (input  (do
            (effect Bail (op stop (-> Int64 Int64)))
            (effect Log (op note (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Log (list)
                ((note (v) s (resume (List.len s) (List.push s v))))
                (+ (* 100 (Log.note n))
                   (+ (* 10 (handle Bail 0
                              ((stop (v) s (* v 2)))
                              (+ 999 (+ (Log.note 7) (Bail.stop 3)))))
                      (Log.note 8)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 62 Int64)))
