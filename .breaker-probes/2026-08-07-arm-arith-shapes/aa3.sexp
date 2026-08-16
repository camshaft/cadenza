(case "aa3 REMAINDER arithmetic in the resume value — (% s 7) cycles as the +5 stride wraps the modulus"
  (input  (do
            (effect E (op g (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((g () s (resume (% s 7) (+ s 5))))
                (+ (E.g) (+ (* 10 (E.g)) (* 100 (E.g))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 613 Int64))
  (call   main (: 0 Int64)) (output (: 350 Int64))
  (call   main (: 10 Int64)) (output (: 613 Int64)))
