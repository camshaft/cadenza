(case "hi1 an arm INSTALLS a fresh handle and resumes with its result — a whole handler lifecycle inside one dispatch"
  (input  (do
            (effect O (op boost (-> Int64)) (op next (-> Int64)))
            (effect J (op get (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((boost () s (resume (+ (handle J 3 ((get () t (resume t t))) (J.get)) s) s))
                 (next () s (resume s (+ s 1))))
                (+ (* 10 (O.boost)) (O.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 85 Int64))
  (call   main (: 0 Int64)) (output (: 30 Int64))
  (call   main (: -2 Int64)) (output (: 8 Int64)))
