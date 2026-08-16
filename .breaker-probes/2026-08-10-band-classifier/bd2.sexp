(case "bd2 the scoring CLOSURE captures a DRAW — the weight itself comes from the thread before the classified reads"
  (input  (do
            (type Band (Mid Int64) (Hi Int64))
            (effect E (op next (-> Int64)) (op probe (-> Band)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 6)))
                 (probe () s (resume (if (< s 10) (Band.Mid s) (Band.Hi s)) (+ s 6))))
                (let ((w (E.next)))
                  (let ((score (fn ((: b Band))
                                 (match b
                                   ((Band.Mid x) (* w x))
                                   ((Band.Hi x) (+ w x))))))
                    (+ (score (E.probe)) (score (E.probe)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 12 Int64))
  (call   main (: 5 Int64)) (output (: 38 Int64))
  (call   main (: -7 Int64)) (output (: -28 Int64)))
