(case "nr1 arm-interned symbols keep content identity ACROSS dispatches — same content = same symbol"
  (input  (do
            (effect St (op tag (-> Int64 Symbol)))
            (def (main (: n Int64))
              (handle St 0
                ((tag (k) s (resume (Symbol.of (String.concat "id-" (if (> k 0) "hi" "lo"))) (+ s 1))))
                (let ((a (St.tag n)))
                  (let ((b (St.tag 0)))
                    (let ((c (St.tag 7)))
                      (+ (* 100 (if (= a c) 1 0))
                         (+ (* 10 (if (= a b) 1 0))
                            (if (< a b) 1 0))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 101 Int64)))
