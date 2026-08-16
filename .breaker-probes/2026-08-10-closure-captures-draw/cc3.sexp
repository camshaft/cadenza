(case "cc3 a closure built in a BRANCH captures that branch's draw — the other branch builds a different closure"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((f (if (= (% n 2) 0)
                             (let ((a (St.get))) (fn (k) (+ (* 100 a) k)))
                             (let ((b (St.get))) (fn (k) (+ (* 1000 b) k))))))
                  (+ (f 7) (St.get)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 412 Int64))
  (call   main (: 5 Int64)) (output (: 5013 Int64)))
