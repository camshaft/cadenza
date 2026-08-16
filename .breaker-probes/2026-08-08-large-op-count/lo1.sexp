(case "lo1 THIRTY dispatches in one region — the fold scales past the corpus's usual handful, arithmetic sum exact"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (St.next))))))))))))))))))))))))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 585 Int64))
  (call   main (: 0 Int64)) (output (: 435 Int64)))
