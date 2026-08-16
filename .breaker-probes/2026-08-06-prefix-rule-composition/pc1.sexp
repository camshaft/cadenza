(case "pc1 TWO different two-site ops in one handler, each with its own prefix segment"
  (input  (do
            (effect St (op siftA (-> Int64 Int64)) (op siftB (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((siftA (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (siftB (v) s (if (> v 5) (resume (* v 2) (+ s 10)) (resume -1 s))))
                (+ (St.siftA 20) (+ (St.siftA 3) (+ (St.siftB 7) (St.siftB 4))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 33 Int64)))
