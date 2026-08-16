(case "zi1 ZERO threads every effect slot (zero seed, zero args, zero results)"
  (input  (do
            (effect St (op echo (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((echo (v) s (resume (+ v s) s)))
                (+ (St.echo 0) (+ (St.echo (- n n)) 7))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7 Int64)))
