(case "a handler ARM body installs its own inner handle around the resume computation"
  (doc    "A handle INSIDE a handler arm: Outer's `ask` arm computes its resume VALUE under a fresh
           inner handle (`Inner.boost` reads the inner seed 50), composing the arm's own state `s`
           with an inner-handled perform — first ask resumes 3+50=53 (state→4), second resumes
           4+50=54 → 10·53+54 = 584. The arm-position handle-install face: the inner handler exists
           only for the arm's evaluation and must not capture Outer's frame or leak into the resumed
           body (a per-resume re-install that lost Outer's state advance gives 10·53+53; an inner
           handle that discharged Outer's next ask gives a CDZ0401 or a wrong route). The
           handle-in-arm companion of the seed-position (:3891) and interposer (:866) compositions.")
  (input  (do
            (effect Outer (op ask (-> Unit Int64)))
            (effect Inner (op boost (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Outer n
                ((ask (u) s
                  (resume (handle Inner 50
                            ((boost (u2) t (resume t (+ t 1))))
                            (+ s (Inner.boost)))
                          (+ s 1))))
                (+ (* 10 (Outer.ask)) (Outer.ask))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 584 Int64)))
