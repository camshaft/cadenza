(case "iwr1 a SIGNED Int8 handler state — wrapping-add crosses the sign boundary through the thread, the widened answers expose the sign-extension"
  (input  (do
            (effect S (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (Int8.wrap n)
                ((add (v) s
                  (let ((s2 (Int8.wrapping-add s (Int8.wrap v))))
                    (resume (Int64.of s2) s2))))
                (let ((a (S.add 100)))
                  (let ((b (S.add 100)))
                    (+ (* 1000 a) b)))))
            (export main)))
  (call   main (: 100 Int64)) (output (: -55956 Int64))
  (call   main (: -28 Int64)) (output (: 71916 Int64)))
