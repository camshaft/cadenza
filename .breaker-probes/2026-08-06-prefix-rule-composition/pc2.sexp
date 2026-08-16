(case "pc2 A-B-A interleave of TWO two-site ops folds (both arms multi-site, no single-op prefix)"
  (input  (do
            (effect St (op siftA (-> Int64 Int64)) (op siftB (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((siftA (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (siftB (v) s (if (> v 5) (resume (* v 2) (+ s 10)) (resume -1 s))))
                (+ (St.siftA 20) (+ (St.siftB 7) (St.siftA 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 64 Int64)))
