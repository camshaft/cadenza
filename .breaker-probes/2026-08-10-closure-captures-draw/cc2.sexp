(case "cc2 a closure whose BODY performs is invoked twice — each invocation draws fresh, capture stays fixed"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((base (St.get)))
                  (let ((f (fn (w) (+ (* w base) (St.get)))))
                    (+ (f 10) (* 1000 (f 10)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 57056 Int64))
  (call   main (: 0 Int64)) (output (: 2001 Int64)))
