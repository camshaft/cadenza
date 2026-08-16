(case "at3 the shadow stack UNWINDS: after the inner handle closes, performs reach the outer directly"
  (input  (do
            (effect E (op e (-> Unit Int64)))
            (def (main (: k Int64))
              (handle E 100 ((e (u) s (resume s (+ s 1))))
                (+ (handle E 7 ((e (u) s (resume (* 10 (E.e)) s)))
                     (E.e))
                   (E.e))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1101 Int64)))
