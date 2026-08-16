(case "gr1 TWO guarded arms on a perform-result scrutinee, both fallbacks perform (post-ag5 radius)"
  (input  (do
            (effect St (op roll (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((roll (u) s (resume s (+ s 3))))
                (match (St.roll)
                  ((guard v (> v 20)) (* v 1000))
                  ((guard v (> v 6)) (* v 100))
                  (v (+ (* 10 (St.roll)) v)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 85 Int64)))
