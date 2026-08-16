(case "nv2 i64 BOUNDARY state: seeded at MAX, arm wraps via guarded arithmetic (no overflow trap in the state slot)"
  (input  (do
            (effect St (op step (-> Unit Int64)))
            (def (main)
              (handle St 9223372036854775807
                ((step (u) s (resume (if (> s 0) 1 0) (if (> s 0) (- 0 s) (+ s 1)))))
                (+ (* 100 (St.step)) (+ (* 10 (St.step)) (St.step)))))
            (export main)))
  (output (: 100 Int64)))
