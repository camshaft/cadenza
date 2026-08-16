(case "nx4 an alternating-sign GEOMETRIC stride (*-2) — the sign flips per dispatch and the sum telescopes"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s -2))))
                (+ (St.next) (+ (St.next) (+ (St.next) (St.next))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: -15 Int64))
  (call   main (: -1 Int64)) (output (: 5 Int64)))
