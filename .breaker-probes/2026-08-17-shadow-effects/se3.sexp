(case "se3 a closure captures the OUTER binder while an inner same-name shadow of another type is live"
  (input  (do
            (def (main (: x Int64))
              (do
                (def f (fn ((: d Int64)) (+ x d)))
                (def r (let ((x "shadow")) (+ (String.len x) (f 10))))
                (+ r x)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 26 Int64)))
