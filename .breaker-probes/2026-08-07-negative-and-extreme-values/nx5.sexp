(case "nx5 i64::MIN-adjacent op arguments — the arm's subtraction stays in range and the two dispatches differ by exactly the state stride"
  (input  (do
            (effect St (op keep (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((keep (v) s (resume (- v s) (+ s 1))))
                (- (St.keep -9223372036854775800) (St.keep -9223372036854775800))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
