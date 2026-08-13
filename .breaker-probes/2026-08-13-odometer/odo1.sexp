(case "odo1 a BASE-100 ODOMETER — the list of wheels absorbs each tick with a CASCADING CARRY that rebuilds cells by List.update and GROWS the list when the top wheel overflows"
  (input  (do
            (effect S (op tick (-> Int64 Int64)))
            (def (carry-add (: w (List Int64)) (: i Int64) (: c Int64))
              (if (= c 0)
                  w
                  (if (= i (List.len w))
                      (carry-add (List.push w 0) i c)
                      (let ((t (+ (match (List.at w i) ((Some x) x) ((None u) 0)) c)))
                        (carry-add (List.update w i (% t 100)) (+ i 1) (/ t 100))))))
            (def (main (: n Int64))
              (handle S (list n)
                ((tick (k) w
                  (let ((w2 (carry-add w 0 k)))
                    (resume (+ (* 100 (List.len w2))
                               (match (List.at w2 0) ((Some x) x) ((None u) -1)))
                            w2))))
                (let ((a (S.tick 50)))
                  (let ((b (S.tick 60)))
                    (let ((c (S.tick 9990)))
                      (+ (* 1000 (+ (* 1000 a) b)) c))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 153213303 Int64))
  (call   main (: 95 Int64)) (output (: 245205395 Int64)))
