(case "ob3c control: BRANCHING arm resuming scalar Option (no Bytes)"
  (input  (do
            (effect Src (op read (-> Int64 (Option Int64))))
            (def (main (: n Int64))
              (handle Src 0
                ((read (v) s
                  (if (> v 0) (resume (Option.Some v) s) (resume (Option.None) s))))
                (+ (match (Src.read n) ((Option.Some x) x) ((Option.None) -1))
                   (* 10 (match (Src.read (- 0 n)) ((Option.Some _x) 1) ((Option.None) 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 25 Int64)))
