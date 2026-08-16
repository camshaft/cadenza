(case "bd1 the arm CLASSIFIES its state into a tri-band SUM — the body matches Lo/Mid/Hi per dispatch as the thread climbs through the bands"
  (input  (do
            (type Band (Lo Int64) (Mid Int64) (Hi Int64))
            (effect E (op probe (-> Band)))
            (def (main (: n Int64))
              (handle E n
                ((probe () s
                  (resume (if (< s 0) (Band.Lo s)
                              (if (< s 10) (Band.Mid s) (Band.Hi s)))
                          (+ s 6))))
                (let ((score (fn ((: b Band))
                               (match b
                                 ((Band.Lo x) (- 0 x))
                                 ((Band.Mid x) (* 10 x))
                                 ((Band.Hi x) (+ x 1000))))))
                  (+ (score (E.probe)) (+ (score (E.probe)) (score (E.probe)))))))
            (export main)))
  (call   main (: -4 Int64)) (output (: 104 Int64))
  (call   main (: 2 Int64)) (output (: 1114 Int64))
  (call   main (: 8 Int64)) (output (: 2114 Int64)))
