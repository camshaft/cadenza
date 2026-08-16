(case "uv4 a RELATIONAL @ensures (ret > x) over a TWO-draw body — the postcondition compares the effectful result to the arg, twice"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (@ (ensures (> ret x)) (def (above (: x Int64)) (+ x (+ (St.next) (St.next)))))
            (def (main (: n Int64))
              (handle St 1
                ((next (u) s (resume s (+ s 1))))
                (+ (above n) (above 100))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 115 Int64))
  (call   main (: 0 Int64)) (output (: 110 Int64)))
