(case "sc1 STRING literal-arm dispatch on a draw — the hi arm re-performs and measures, the lo arm is constant"
  (input  (do
            (effect St (op name (-> Int64 String)))
            (def (main (: n Int64))
              (handle St n
                ((name (k) s (resume (if (> s k) "hi" "lo") (+ s 1))))
                (let ((w (St.name 3)))
                  (match w
                    ("hi" (+ 100 (String.byte-len (St.name 0))))
                    ("lo" 200)
                    (_o 300)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 102 Int64))
  (call   main (: 1 Int64)) (output (: 200 Int64)))
