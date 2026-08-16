(case "sa3 STRING resume values — the arm builds a rope per dispatch branching op-arg vs live state, body concatenates two"
  (input  (do
            (effect Log (op name (-> Int64 String)))
            (def (main (: n Int64))
              (handle Log n
                ((name (k) s (resume (String.concat "u" (if (> k s) "-big" "-sm")) (+ s 1))))
                (+ (String.byte-len (Log.name 3))
                   (* 10 (String.byte-len (String.concat (Log.name 99) (Log.name 0)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 94 Int64))
  (call   main (: 2 Int64)) (output (: 95 Int64)))
