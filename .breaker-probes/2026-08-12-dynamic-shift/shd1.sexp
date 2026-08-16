(case "shd1 DYNAMIC shift amounts from the state — two drawn widths, the value-63 draw traps the checked shift overflow"
  (input  (do
            (effect S (op amt (-> Int64)))
            (def (main (: n Int64))
              (handle S n
                ((amt () s (resume s (+ s 31))))
                (let ((a (<< 1 (S.amt))))
                  (let ((b (<< 1 (S.amt))))
                    (+ a b)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 4294967298 Int64))
  (call   main (: 0 Int64)) (output (: 2147483649 Int64))
  (call   main (: 32 Int64)) (trap "integer overflow"))
