(case "sg4 TWO-SIDED rope growth — the op arg's sign picks append-right vs prepend-left, exact content across three sign patterns"
  (input  (do
            (effect St (op tag (-> Int64 String)))
            (def (main (: n Int64))
              (handle St "M"
                ((tag (side) s (resume s (if (> side 0)
                                             (String.concat s "R")
                                             (String.concat "L" s)))))
                (do
                  (St.tag n)
                  (St.tag (- 0 n))
                  (St.tag n)
                  (let ((w (St.tag 0)))
                    (if (= w "LMRR") 1 (if (= w "LLMR") 2 (if (= w "LLLM") 3 0)))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64))
  (call   main (: -1 Int64)) (output (: 2 Int64))
  (call   main (: 0 Int64)) (output (: 3 Int64)))
