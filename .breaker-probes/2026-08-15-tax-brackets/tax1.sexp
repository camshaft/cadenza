(case "tax1 PROGRESSIVE tax brackets with a seed-shaped edge — assess splits the income into three bands via chained match binders over clamped compounds (ten twenty and thirty percent truncating), accumulating into the audit total, and the WIDER bracket taxes every income LESS while the sub-bracket income taxes to zero on both"
  (input  (do
            (effect T
              (op assess (-> Int64 Int64))
              (op audit (-> Int64)))
            (def (bandtax (: b Int64) (: inc Int64))
              (match (if (< inc b) inc b)
                (lo
                  (match (if (< b inc) (if (< (* 2 b) (- inc b)) (* 2 b) (- inc b)) 0)
                    (mid
                      (match (if (< (* 3 b) inc) (- inc (* 3 b)) 0)
                        (hi
                          (+ (/ lo 10) (+ (/ (* mid 2) 10) (/ (* hi 3) 10))))))))))
            (def (main (: n Int64))
              (handle T (: 0 Int64)
                ((assess (inc) total
                  (match (bandtax (* (+ (% n 4) 2) 10) inc)
                    (tax (resume tax (+ total tax)))))
                 (audit () total (resume total total)))
                (let ((a (T.assess 30)))
                  (let ((b (T.assess 75)))
                    (let ((c (T.assess 150)))
                      (let ((d (T.assess 9)))
                        (let ((e (T.audit)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 3011029000043 Int64))
  (call   main (: 0 Int64)) (output (: 4014037000055 Int64)))
