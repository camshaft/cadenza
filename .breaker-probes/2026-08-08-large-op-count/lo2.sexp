(case "lo2 a NINE-level shadow tower (outer + eight) — one draw per level, the deepest doubling, strides 1-8"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (St.next) (handle St 100 ((next () s (resume s (+ s 1)))) (+ (St.next) (handle St 200 ((next () s (resume s (+ s 2)))) (+ (St.next) (handle St 300 ((next () s (resume s (+ s 3)))) (+ (St.next) (handle St 400 ((next () s (resume s (+ s 4)))) (+ (St.next) (handle St 500 ((next () s (resume s (+ s 5)))) (+ (St.next) (handle St 600 ((next () s (resume s (+ s 6)))) (+ (St.next) (handle St 700 ((next () s (resume s (+ s 7)))) (+ (St.next) (handle St 800 ((next () s (resume s (+ s 8)))) (+ (St.next) (St.next))))))))))))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4413 Int64))
  (call   main (: 0 Int64)) (output (: 4408 Int64)))
