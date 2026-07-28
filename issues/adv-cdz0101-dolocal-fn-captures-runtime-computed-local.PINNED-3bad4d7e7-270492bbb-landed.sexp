(case "c4 fn-body do-local fn captures a RUNTIME-computed local"
  (input (do
        (def (outer (: n Int64))
          (do
            (def m (* n 3))
            (def (inner (: x Int64)) (+ x m))
            (inner n)))
        (def (main (: n Int64)) (outer n))
        (export main)))
  (call main (: 5 Int64)) (output (: 20 Int64)))

(case "c5 same but capture via LET instead of do-def"
  (input (do
        (def (outer (: n Int64))
          (let ((m (* n 3)))
            (do
              (def (inner (: x Int64)) (+ x m))
              (inner n))))
        (def (main (: n Int64)) (outer n))
        (export main)))
  (call main (: 5 Int64)) (output (: 20 Int64)))
(case "c1 top-level do-local fn captures CONSTANT local"
  (input (do (def base 10) (def (addb (: n Int64)) (+ n base)) (def (main (: n Int64)) (addb n)) (export main)))
  (call main (: 5 Int64)) (output (: 15 Int64)))

(case "c2 fn-body do-local fn captures the fn PARAMETER directly"
  (input (do
        (def (outer (: n Int64))
          (do
            (def (inner (: x Int64)) (+ x n))
            (inner 1)))
        (def (main (: n Int64)) (outer n))
        (export main)))
  (call main (: 5 Int64)) (output (: 6 Int64)))

(case "c3 fn-body do-local fn captures a CONSTANT local"
  (input (do
        (def (outer (: n Int64))
          (do
            (def m 3)
            (def (inner (: x Int64)) (+ x m))
            (inner n)))
        (def (main (: n Int64)) (outer n))
        (export main)))
  (call main (: 5 Int64)) (output (: 8 Int64)))
